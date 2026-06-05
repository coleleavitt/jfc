use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::dispatcher::{AuditBountyDispatcher, BountyRunner, ValidationOutcome};
use crate::enumerator::{GraphQuery, SourceEnumerator};
use crate::error::Result;
use crate::reachability::{ReachabilityGraph, ReachabilityProver};
use crate::store::FindingStore;
use crate::suspicious_point::{SuspiciousPoint, SuspiciousPointFinder};
use crate::taint::{TaintGraph, TaintSpecProvider, TaintTracker};
use crate::types::*;

/// Configuration for an audit run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    /// Maximum call-graph depth to explore.
    pub max_depth: usize,
    /// Whether to include findings for unreachable code.
    pub include_unreachable: bool,
    /// Maximum token budget for validator bounties.
    pub max_budget_tokens: u64,
    /// Minimum severity to report.
    pub severity_floor: Severity,
    /// File path scope (prefix filter).
    pub scope: Option<String>,
    /// Whether to skip already-seen findings.
    pub incremental: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            include_unreachable: false,
            max_budget_tokens: 100_000,
            severity_floor: Severity::Low,
            scope: None,
            incremental: true,
        }
    }
}

/// Statistics from an audit run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditStats {
    pub entrypoints_enumerated: usize,
    pub suspicious_points_found: usize,
    pub reachable_count: usize,
    pub taint_chains_traced: usize,
    pub validator_runs: usize,
    pub validated_count: usize,
    pub false_positives_count: usize,
    pub tokens_spent: u64,
    pub walltime_ms: u64,
}

/// Final audit report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub findings: Vec<Finding>,
    pub stats: AuditStats,
}

/// Trait for reading source files.
pub trait SourceReader: Send + Sync {
    /// Read the contents of a source file.
    fn read_file(&self, path: &str) -> Result<String>;

    /// List all source files in scope.
    fn list_files(&self, scope: Option<&str>) -> Result<Vec<String>>;
}

/// The main orchestrator that chains the full audit pipeline.
pub struct AuditOrchestrator<G, R, P, T, B, S>
where
    G: GraphQuery,
    R: ReachabilityGraph,
    P: TaintSpecProvider,
    T: TaintGraph,
    B: BountyRunner,
    S: SourceReader,
{
    config: AuditConfig,
    enumerator: SourceEnumerator<G>,
    reachability: ReachabilityProver<R>,
    taint_tracker: TaintTracker<P, T>,
    dispatcher: AuditBountyDispatcher<B>,
    source_reader: S,
    store: FindingStore,
}

impl<G, R, P, T, B, S> AuditOrchestrator<G, R, P, T, B, S>
where
    G: GraphQuery,
    R: ReachabilityGraph,
    P: TaintSpecProvider,
    T: TaintGraph,
    B: BountyRunner,
    S: SourceReader,
{
    pub fn new(
        config: AuditConfig,
        graph: G,
        reachability_graph: R,
        taint_provider: P,
        taint_graph: T,
        bounty_runner: B,
        source_reader: S,
        store: FindingStore,
    ) -> Self {
        let enumerator = SourceEnumerator::new(graph);
        let reachability = ReachabilityProver::new(reachability_graph);
        let taint_tracker = TaintTracker::new(taint_provider, taint_graph, store.root());
        let dispatcher = AuditBountyDispatcher::new(bounty_runner);

        Self {
            config,
            enumerator,
            reachability,
            taint_tracker,
            dispatcher,
            source_reader,
            store,
        }
    }

    /// Run the full audit pipeline.
    pub async fn run(&mut self) -> Result<AuditReport> {
        let start = Instant::now();
        let mut stats = AuditStats::default();
        let mut findings: Vec<Finding> = Vec::new();

        // 1. Enumerate entry points
        info!("enumerating entry points");
        let entrypoints = self.enumerator.prioritize()?;
        stats.entrypoints_enumerated = entrypoints.len();
        debug!(count = entrypoints.len(), "entry points found");

        // 2. Find suspicious points in source files
        info!("scanning for suspicious points");
        let finder = SuspiciousPointFinder::new();
        let files = self
            .source_reader
            .list_files(self.config.scope.as_deref())?;
        let mut suspicious_points: Vec<SuspiciousPoint> = Vec::new();

        for file_path in &files {
            if let Ok(source) = self.source_reader.read_file(file_path) {
                let points = finder.scan_file(file_path, &source);
                suspicious_points.extend(points);
            }
        }
        stats.suspicious_points_found = suspicious_points.len();
        debug!(count = suspicious_points.len(), "suspicious points found");

        // 3. Reachability filter
        info!("proving reachability");
        let mut reachable_points: Vec<(SuspiciousPoint, Vec<String>, Vec<String>)> = Vec::new();

        for point in &suspicious_points {
            match self.reachability.prove_with_preconditions(&point.handle)? {
                Some(proof) => {
                    reachable_points.push((point.clone(), proof.path, proof.preconditions));
                }
                None => {
                    if self.config.include_unreachable {
                        reachable_points.push((point.clone(), vec![point.handle.clone()], vec![]));
                    }
                }
            }
        }
        stats.reachable_count = reachable_points.len();

        // 4. Taint tracing
        info!("tracing taint flows");
        let _ = self.taint_tracker.discover_specs().await?;
        let taint_chains = self.taint_tracker.trace()?;
        stats.taint_chains_traced = taint_chains.len();

        // 5. Build findings from reachable suspicious points
        let revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for (point, path, preconditions) in &reachable_points {
            let kind = finding_kind_from_trigger(&point.trigger_kind);
            let severity = severity_for_kind(kind);

            if severity < self.config.severity_floor {
                continue;
            }

            let location = SourceSpan {
                file: point.file.clone(),
                start_line: point.region_lines.0,
                end_line: point.region_lines.1,
            };

            let first_path_entry = path.first().map(|s| s.as_str()).unwrap_or("");
            let id = Finding::compute_id(kind, &location, first_path_entry);

            // Check for associated taint chain
            let taint_chain = taint_chains
                .iter()
                .find(|c| c.hops.iter().any(|h| h.to_symbol == point.handle))
                .map(|c| c.hops.clone());

            let finding = Finding {
                id,
                severity,
                kind,
                location,
                granularity: Granularity::SuspiciousPoint {
                    lines: point.region_lines,
                },
                reachability_path: path.clone(),
                taint_chain,
                preconditions: preconditions.clone(),
                validator_verdicts: vec![],
                poc_status: PocStatus::NotAttempted,
                first_seen_revision: revision,
                last_seen_revision: revision,
                suppressed: None,
            };

            findings.push(finding);
        }

        // Also add pure taint findings (source→sink without pre-existing suspicious point)
        for chain in &taint_chains {
            if chain.sanitized {
                continue;
            }
            let location = SourceSpan {
                file: "unknown".to_string(),
                start_line: 0,
                end_line: 0,
            };
            let path = chain
                .hops
                .iter()
                .map(|h| h.from_symbol.clone())
                .collect::<Vec<_>>();
            let first_path_entry = path.first().map(|s| s.as_str()).unwrap_or("");
            let id = Finding::compute_id(FindingKind::TaintedSink, &location, first_path_entry);

            // Avoid duplicates
            if findings.iter().any(|f| f.id == id) {
                continue;
            }

            findings.push(Finding {
                id,
                severity: Severity::Critical,
                kind: FindingKind::TaintedSink,
                location,
                granularity: Granularity::Function,
                reachability_path: path,
                taint_chain: Some(chain.hops.clone()),
                preconditions: vec![],
                validator_verdicts: vec![],
                poc_status: PocStatus::NotAttempted,
                first_seen_revision: revision,
                last_seen_revision: revision,
                suppressed: None,
            });
        }

        // 6. Dedup and merge with store
        info!(count = findings.len(), "merging findings");
        for finding in &findings {
            self.store.append(finding.clone())?;
        }

        // 7. Validate via bounty economy (budget-limited)
        info!("dispatching validation bounties");
        let mut to_validate: Vec<Finding> = findings
            .iter()
            .filter(|f| f.severity >= self.config.severity_floor)
            .cloned()
            .collect();

        if !to_validate.is_empty() {
            let outcomes = self.dispatcher.validate_batch(&mut to_validate).await?;
            for outcome in outcomes.iter() {
                stats.validator_runs += 1;
                match outcome {
                    ValidationOutcome::Validated => stats.validated_count += 1,
                    ValidationOutcome::FalsePositive => stats.false_positives_count += 1,
                    ValidationOutcome::BudgetExhausted => break,
                    _ => {}
                }
            }

            // Update findings with validation results
            for validated in &to_validate {
                if let Some(f) = findings.iter_mut().find(|f| f.id == validated.id) {
                    f.validator_verdicts = validated.validator_verdicts.clone();
                    f.poc_status = validated.poc_status;
                }
            }
        }

        // 8. Persist final state
        for finding in &findings {
            self.store.append(finding.clone())?;
        }

        stats.walltime_ms = start.elapsed().as_millis() as u64;

        info!(
            findings = findings.len(),
            validated = stats.validated_count,
            walltime_ms = stats.walltime_ms,
            "audit complete"
        );

        Ok(AuditReport { findings, stats })
    }
}

/// Map trigger kind to finding kind.
fn finding_kind_from_trigger(trigger: &crate::suspicious_point::TriggerKind) -> FindingKind {
    use crate::suspicious_point::TriggerKind;
    match trigger {
        TriggerKind::UnsafeBlock | TriggerKind::UnsafeTransmute | TriggerKind::RawPointer => {
            FindingKind::ResourceLeak
        }
        TriggerKind::Unwrap
        | TriggerKind::Expect
        | TriggerKind::Panic
        | TriggerKind::Unreachable => FindingKind::UnreachablePanic,
        TriggerKind::ArrayIndex => FindingKind::MissingBoundsCheck,
        TriggerKind::FfiCall => FindingKind::TaintedSink,
        TriggerKind::TaintedLoop => FindingKind::InvariantViolation,
    }
}

/// Default severity for a finding kind.
fn severity_for_kind(kind: FindingKind) -> Severity {
    match kind {
        FindingKind::TaintedSink => Severity::Critical,
        FindingKind::ResourceLeak => Severity::High,
        FindingKind::MissingBoundsCheck => Severity::High,
        FindingKind::UnreachablePanic => Severity::Medium,
        FindingKind::InvariantViolation => Severity::Medium,
        FindingKind::RaceCondition => Severity::High,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::MarketHealth;
    use crate::enumerator::{EntryPoint, EntryPointKind};
    use crate::taint::*;
    use async_trait::async_trait;
    use tempfile::TempDir;

    // ---- Mock implementations ----

    struct MockGraphQuery;
    impl GraphQuery for MockGraphQuery {
        fn entrypoints(&self) -> Result<Vec<EntryPoint>> {
            Ok(vec![EntryPoint {
                handle: "fn:main".to_string(),
                kind: EntryPointKind::Main,
                reachable_count: 10,
                public_signature: "fn main()".to_string(),
                file: "src/main.rs".to_string(),
                line: 1,
            }])
        }

        fn reachable_count(&self, _handle: &str) -> Result<usize> {
            Ok(10)
        }
    }

    struct MockReachabilityGraph;
    impl ReachabilityGraph for MockReachabilityGraph {
        fn find_path_to(&self, target: &str) -> Result<Option<Vec<String>>> {
            Ok(Some(vec!["fn:main".to_string(), target.to_string()]))
        }

        fn find_path_with_preconditions(
            &self,
            target: &str,
        ) -> Result<Option<(Vec<String>, Vec<String>, EntryPointKind)>> {
            Ok(Some((
                vec!["fn:main".to_string(), target.to_string()],
                vec![],
                EntryPointKind::Main,
            )))
        }
    }

    struct MockTaintProvider;
    #[async_trait]
    impl TaintSpecProvider for MockTaintProvider {
        async fn discover_specs(&self) -> Result<TaintSpecs> {
            Ok(TaintSpecs::default())
        }
    }

    struct MockTaintGraph;
    impl TaintGraph for MockTaintGraph {
        fn trace(&self, _source: &str, _param: &str) -> Result<Vec<TaintHop>> {
            Ok(vec![])
        }
    }

    struct MockBountyRunner {
        healthy: bool,
    }
    #[async_trait]
    impl BountyRunner for MockBountyRunner {
        async fn validate_finding(&self, _finding: &Finding) -> Result<ValidationOutcome> {
            Ok(ValidationOutcome::Validated)
        }

        async fn market_health(&self) -> Result<MarketHealth> {
            Ok(MarketHealth {
                score: if self.healthy { 0.8 } else { 0.1 },
                is_healthy: self.healthy,
            })
        }
    }

    struct MockSourceReader;
    impl SourceReader for MockSourceReader {
        fn read_file(&self, _path: &str) -> Result<String> {
            Ok(r#"
pub fn process(input: &str) {
    let cmd = input.to_string();
    let _ = cmd.as_str().unwrap();
}
"#
            .to_string())
        }

        fn list_files(&self, _scope: Option<&str>) -> Result<Vec<String>> {
            Ok(vec!["src/lib.rs".to_string()])
        }
    }

    #[allow(dead_code)]
    struct EmptySourceReader;
    impl SourceReader for EmptySourceReader {
        fn read_file(&self, _path: &str) -> Result<String> {
            Ok(String::new())
        }

        fn list_files(&self, _scope: Option<&str>) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn full_pipeline_with_mocks_normal() {
        let tmp = TempDir::new().unwrap();
        let store = FindingStore::open_project(tmp.path()).unwrap();
        let config = AuditConfig::default();

        let mut orchestrator = AuditOrchestrator::new(
            config,
            MockGraphQuery,
            MockReachabilityGraph,
            MockTaintProvider,
            MockTaintGraph,
            MockBountyRunner { healthy: true },
            MockSourceReader,
            store,
        );

        let report = orchestrator.run().await.unwrap();
        assert!(report.stats.entrypoints_enumerated > 0);
        assert!(report.stats.suspicious_points_found > 0);
        assert!(report.stats.reachable_count > 0);
        assert!(!report.findings.is_empty());
    }

    #[tokio::test]
    async fn budget_exhausted_partial_report_robust() {
        let tmp = TempDir::new().unwrap();
        let store = FindingStore::open_project(tmp.path()).unwrap();
        let config = AuditConfig {
            max_budget_tokens: 0,
            ..Default::default()
        };

        let mut orchestrator = AuditOrchestrator::new(
            config,
            MockGraphQuery,
            MockReachabilityGraph,
            MockTaintProvider,
            MockTaintGraph,
            MockBountyRunner { healthy: false }, // unhealthy = budget gate
            MockSourceReader,
            store,
        );

        let report = orchestrator.run().await.unwrap();
        // Should still produce findings even without validation
        assert!(report.stats.suspicious_points_found > 0);
        // But no validated findings (budget exhausted)
        assert_eq!(report.stats.validated_count, 0);
    }
}

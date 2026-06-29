use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use super::{
    ApplyToStore, RsiCurator, RsiCuratorConfig, RsiCuratorJob, RsiLoopSandboxPlan,
    RsiPromotionPolicy, RsiSandboxEnforcement, RsiTrace,
};
use crate::LearnError;

#[derive(Debug, Clone)]
pub struct RsiCuratorWorkerConfig {
    pub binary: PathBuf,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsiWorkerInput {
    pub traces: Vec<RsiTrace>,
    pub config: RsiCuratorConfig,
    pub promotion_policy: RsiPromotionPolicy,
    pub project_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsiWorkerOutput {
    pub actions: usize,
    pub traces_scored: usize,
    pub candidates_seen: usize,
}

pub fn run_rsi_worker_job(job: &RsiCuratorJob) -> Result<RsiWorkerOutput, LearnError> {
    let _linkscope_job = linkscope::phase("learn.rsi_worker.run_job");
    linkscope::event_fields(
        "learn.rsi_worker.run_job",
        [
            linkscope::TraceField::count(
                "traces",
                u64::try_from(job.traces.len()).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count("has_worker", u64::from(job.worker.is_some())),
            linkscope::TraceField::count(
                "has_sandbox",
                u64::from(job.sandbox_enforcement.is_some()),
            ),
            linkscope::TraceField::count("has_project_key", u64::from(job.project_key.is_some())),
        ],
    );
    let Some(worker) = &job.worker else {
        return Err(LearnError::ContractViolation {
            message: "RSI worker config missing".to_owned(),
        });
    };
    let Some(sandbox) = &job.sandbox_enforcement else {
        return Err(LearnError::ContractViolation {
            message: "RSI worker sandbox receipt missing".to_owned(),
        });
    };
    if !sandbox.kernel_enforced || !sandbox.egress_isolated {
        return Err(LearnError::ContractViolation {
            message: "RSI worker requires kernel-enforced egress isolation".to_owned(),
        });
    }
    let input = RsiWorkerInput {
        traces: job.traces.clone(),
        config: job.config.clone(),
        promotion_policy: job.promotion_policy.clone(),
        project_key: job.project_key.clone(),
    };
    run_bwrap_worker(worker, &input)
}

pub async fn run_rsi_worker_file(input: &Path, output: &Path) -> Result<(), LearnError> {
    let _linkscope_file = linkscope::phase("learn.rsi_worker.run_file");
    linkscope::event_fields(
        "learn.rsi_worker.run_file",
        [
            linkscope::TraceField::text("input", input.display().to_string()),
            linkscope::TraceField::text("output", output.display().to_string()),
        ],
    );
    let raw = std::fs::read(input)?;
    linkscope::record_bytes(
        "learn.rsi_worker.input_bytes",
        u64::try_from(raw.len()).unwrap_or(u64::MAX),
    );
    let input = serde_json::from_slice::<RsiWorkerInput>(&raw)?;
    let output_body = run_worker_input(input).await?;
    let raw_output = serde_json::to_vec_pretty(&output_body)?;
    linkscope::record_bytes(
        "learn.rsi_worker.output_bytes",
        u64::try_from(raw_output.len()).unwrap_or(u64::MAX),
    );
    std::fs::write(output, raw_output)?;
    Ok(())
}

async fn run_worker_input(input: RsiWorkerInput) -> Result<RsiWorkerOutput, LearnError> {
    let _linkscope_input = linkscope::phase("learn.rsi_worker.run_input");
    linkscope::event_fields(
        "learn.rsi_worker.run_input",
        [
            linkscope::TraceField::count(
                "traces",
                u64::try_from(input.traces.len()).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count("project_key", u64::from(input.project_key.is_some())),
        ],
    );
    let curator = RsiCurator::new(input.config, input.promotion_policy);
    let mut report = curator.run(&input.traces)?;
    report.experiment_job.external_worker_sandbox =
        RsiSandboxEnforcement::bubblewrap_worker(&RsiLoopSandboxPlan::default(), true, true);
    let actions = if let Some(project_key) = &input.project_key {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        report.apply_to_store(&store, project_key).await?.actions()
    } else {
        report.len()
    };
    linkscope::event_fields(
        "learn.rsi_worker.run_input.result",
        [
            linkscope::TraceField::count("actions", u64::try_from(actions).unwrap_or(u64::MAX)),
            linkscope::TraceField::count(
                "traces_scored",
                u64::try_from(report.traces_scored).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count(
                "candidates_seen",
                u64::try_from(report.candidates.len()).unwrap_or(u64::MAX),
            ),
        ],
    );
    Ok(RsiWorkerOutput {
        actions,
        traces_scored: report.traces_scored,
        candidates_seen: report.candidates.len(),
    })
}

fn run_bwrap_worker(
    worker: &RsiCuratorWorkerConfig,
    input: &RsiWorkerInput,
) -> Result<RsiWorkerOutput, LearnError> {
    let _linkscope_bwrap = linkscope::phase("learn.rsi_worker.run_bwrap");
    linkscope::event_fields(
        "learn.rsi_worker.run_bwrap",
        [
            linkscope::TraceField::text("binary", worker.binary.display().to_string()),
            linkscope::TraceField::text("cwd", worker.cwd.display().to_string()),
            linkscope::TraceField::count(
                "traces",
                u64::try_from(input.traces.len()).unwrap_or(u64::MAX),
            ),
        ],
    );
    let dir = worker.cwd.join(".jfc").join("rsi-worker");
    std::fs::create_dir_all(&dir)?;
    let nonce = uuid::Uuid::new_v4();
    let input_path = dir.join(format!("{nonce}.input.json"));
    let output_path = dir.join(format!("{nonce}.output.json"));
    let raw_input = serde_json::to_vec_pretty(input)?;
    linkscope::record_bytes(
        "learn.rsi_worker.bwrap_input_bytes",
        u64::try_from(raw_input.len()).unwrap_or(u64::MAX),
    );
    std::fs::write(&input_path, raw_input)?;
    let _linkscope_command = linkscope::phase("learn.rsi_worker.bwrap_command");
    let status = worker_command(worker, &input_path, &output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()?;
    if !status.success() {
        linkscope::event_fields(
            "learn.rsi_worker.run_bwrap.result",
            [
                linkscope::TraceField::text("status", "failed"),
                linkscope::TraceField::signed("exit", i64::from(status.code().unwrap_or(-1))),
            ],
        );
        return Err(LearnError::ContractViolation {
            message: format!("RSI worker exited with status {status}"),
        });
    }
    let raw = std::fs::read(&output_path)?;
    linkscope::record_bytes(
        "learn.rsi_worker.bwrap_output_bytes",
        u64::try_from(raw.len()).unwrap_or(u64::MAX),
    );
    linkscope::event_fields(
        "learn.rsi_worker.run_bwrap.result",
        [linkscope::TraceField::text("status", "ok")],
    );
    serde_json::from_slice::<RsiWorkerOutput>(&raw).map_err(Into::into)
}

fn worker_command(worker: &RsiCuratorWorkerConfig, input: &Path, output: &Path) -> Command {
    let _linkscope_command = linkscope::phase("learn.rsi_worker.worker_command");
    let mut cmd = Command::new("bwrap");
    cmd.args(bwrap_args(worker, input, output));
    cmd
}

fn bwrap_args(worker: &RsiCuratorWorkerConfig, input: &Path, output: &Path) -> Vec<String> {
    let _linkscope_args = linkscope::phase("learn.rsi_worker.bwrap_args");
    let cwd = worker.cwd.display().to_string();
    let binary = worker.binary.display().to_string();
    let mut args = vec![
        "--ro-bind".into(),
        "/usr".into(),
        "/usr".into(),
        "--ro-bind".into(),
        "/etc".into(),
        "/etc".into(),
        "--ro-bind".into(),
        "/bin".into(),
        "/bin".into(),
        "--ro-bind".into(),
        "/lib".into(),
        "/lib".into(),
        "--ro-bind-try".into(),
        "/lib64".into(),
        "/lib64".into(),
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--bind".into(),
        cwd.clone(),
        cwd.clone(),
        "--chdir".into(),
        cwd,
        "--unshare-net".into(),
    ];
    if let Some(parent) = worker.binary.parent() {
        let parent = parent.display().to_string();
        args.extend(["--ro-bind-try".into(), parent.clone(), parent]);
    }
    if let Ok(db_path) = std::env::var("JFC_KNOWLEDGE_DB")
        && let Some(parent) = Path::new(&db_path).parent()
    {
        let parent = parent.display().to_string();
        args.extend(["--bind-try".into(), parent.clone(), parent]);
    }
    args.extend([
        binary,
        "rsi-worker".into(),
        "--input".into(),
        input.display().to_string(),
        "--output".into(),
        output.display().to_string(),
    ]);
    linkscope::event_fields(
        "learn.rsi_worker.bwrap_args.result",
        [linkscope::TraceField::count(
            "args",
            u64::try_from(args.len()).unwrap_or(u64::MAX),
        )],
    );
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bwrap_args_unshare_network_and_bind_worker_binary_normal() {
        let temp = tempfile::tempdir().unwrap();
        let worker = RsiCuratorWorkerConfig {
            binary: temp.path().join("jfc"),
            cwd: temp.path().to_path_buf(),
        };

        let args = bwrap_args(
            &worker,
            &temp.path().join("input.json"),
            &temp.path().join("output.json"),
        );

        assert!(args.iter().any(|arg| arg == "--unshare-net"));
        assert!(args.iter().any(|arg| arg == "rsi-worker"));
        assert!(args.iter().any(|arg| arg == "--input"));
        assert!(args.iter().any(|arg| arg == "--output"));
    }

    #[tokio::test]
    async fn worker_file_runs_curator_and_writes_output_normal() {
        let temp = tempfile::tempdir().unwrap();
        let input = RsiWorkerInput {
            traces: vec![RsiTrace {
                session_id: "manual".to_owned(),
                thinking_blocks: vec!["private reasoning summary".to_owned()],
                thinking_tokens: 100,
                tool_steps: vec![
                    super::super::RsiToolStep::new("Edit", false),
                    super::super::RsiToolStep::new("Read", true),
                    super::super::RsiToolStep::new("Edit", true),
                ],
                outcome: Some(super::super::RsiOutcome::UserCorrected),
                user_correction: Some("verify first".to_owned()),
                ..RsiTrace::default()
            }],
            config: RsiCuratorConfig::default(),
            promotion_policy: RsiPromotionPolicy::default(),
            project_key: None,
        };
        let input_path = temp.path().join("input.json");
        let output_path = temp.path().join("output.json");
        std::fs::write(&input_path, serde_json::to_vec(&input).unwrap()).unwrap();

        run_rsi_worker_file(&input_path, &output_path)
            .await
            .unwrap();

        let output: RsiWorkerOutput =
            serde_json::from_slice(&std::fs::read(&output_path).unwrap()).unwrap();
        assert_eq!(output.traces_scored, 1);
        assert!(output.candidates_seen > 0);
        assert!(output.actions > 0);
    }
}

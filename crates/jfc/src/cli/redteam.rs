//! `jfc redteam run` — controlled post-PAIR red-team algorithm runner.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use clap::{Args, Subcommand, ValueEnum};
use jfc_audit::{
    AuditError, RedTeamConfig, RedTeamHeuristicJudge, RedTeamMethod, RedTeamRun, RedTeamRunner,
    RewriteModel,
};
use jfc_provider::{Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

use crate::runtime::bootstrap::{build_providers, resolve_provider_model};

#[derive(Subcommand, Debug)]
pub(super) enum RedTeamSubcommand {
    /// Run controlled PAIR-descendant and theory-grounded red-team methods.
    Run(RedTeamRunArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RedTeamMethodArg {
    Tap,
    Autodan,
    Crescendo,
    Goat,
    Robopair,
    Proact,
    PairMarkov,
    TapUcb,
    Boja,
    Casp,
    #[value(name = "j-rl", alias = "jrl")]
    Jrl,
    #[value(name = "pot-j", alias = "potj")]
    PotJ,
    #[value(name = "soc-prompt", alias = "socprompt")]
    SocPrompt,
}

impl From<RedTeamMethodArg> for RedTeamMethod {
    fn from(value: RedTeamMethodArg) -> Self {
        match value {
            RedTeamMethodArg::Tap => RedTeamMethod::Tap,
            RedTeamMethodArg::Autodan => RedTeamMethod::AutoDan,
            RedTeamMethodArg::Crescendo => RedTeamMethod::Crescendo,
            RedTeamMethodArg::Goat => RedTeamMethod::Goat,
            RedTeamMethodArg::Robopair => RedTeamMethod::RoboPair,
            RedTeamMethodArg::Proact => RedTeamMethod::ProAct,
            RedTeamMethodArg::PairMarkov => RedTeamMethod::PairMarkov,
            RedTeamMethodArg::TapUcb => RedTeamMethod::TapUcb,
            RedTeamMethodArg::Boja => RedTeamMethod::Boja,
            RedTeamMethodArg::Casp => RedTeamMethod::Casp,
            RedTeamMethodArg::Jrl => RedTeamMethod::Jrl,
            RedTeamMethodArg::PotJ => RedTeamMethod::PotJ,
            RedTeamMethodArg::SocPrompt => RedTeamMethod::SocPrompt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum JudgeKind {
    Model,
    Heuristic,
}

#[derive(Args, Debug)]
pub(super) struct RedTeamRunArgs {
    /// Objective to evaluate. If omitted, read from stdin.
    #[arg(value_name = "OBJECTIVE")]
    objective: Option<String>,

    /// Explicit opt-in required unless `[redteam] enabled = true` is set.
    #[arg(long = "allow-red-team")]
    allow_red_team: bool,

    /// Algorithm to run.
    #[arg(long = "method", value_enum)]
    method: Option<RedTeamMethodArg>,

    /// Desired beginning for a successful target response.
    #[arg(long = "target-start", value_name = "TEXT")]
    target_start: Option<String>,

    #[arg(long = "attacker-model", value_name = "MODEL")]
    attacker_model: Option<String>,
    #[arg(long = "target-model", value_name = "MODEL")]
    target_model: Option<String>,
    #[arg(long = "judge-model", value_name = "MODEL")]
    judge_model: Option<String>,
    #[arg(long = "attacker-provider", value_name = "NAME")]
    attacker_provider: Option<String>,
    #[arg(long = "target-provider", value_name = "NAME")]
    target_provider: Option<String>,
    #[arg(long = "judge-provider", value_name = "NAME")]
    judge_provider: Option<String>,
    #[arg(long = "judge", value_enum)]
    judge: Option<JudgeKind>,

    #[arg(long = "n-streams", value_name = "N")]
    n_streams: Option<usize>,
    #[arg(long = "n-iterations", value_name = "N")]
    n_iterations: Option<usize>,
    #[arg(long = "branch-factor", value_name = "N")]
    branch_factor: Option<usize>,
    #[arg(long = "prune-width", value_name = "N")]
    prune_width: Option<usize>,
    #[arg(long = "population-size", value_name = "N")]
    population_size: Option<usize>,
    #[arg(long = "generations", value_name = "N")]
    generations: Option<usize>,
    #[arg(long = "max-turns", value_name = "N")]
    max_turns: Option<usize>,
    #[arg(long = "success-threshold", value_name = "SCORE")]
    success_threshold: Option<f64>,
    #[arg(long = "proact-defense")]
    proact_defense: bool,
    #[arg(long = "robot-context", value_name = "TEXT")]
    robot_context: Option<String>,
    #[arg(long = "beta0", value_name = "BETA")]
    beta0: Option<f64>,
    #[arg(long = "casp-drift", value_name = "DRIFT")]
    casp_drift: Option<f64>,
    #[arg(long = "embedding-dim", value_name = "N")]
    embedding_dim: Option<usize>,
    #[arg(long = "bo-candidates", value_name = "N")]
    bo_candidates: Option<usize>,
    #[arg(long = "jrl-learning-rate", value_name = "ALPHA")]
    jrl_learning_rate: Option<f64>,
    #[arg(long = "jrl-gamma", value_name = "GAMMA")]
    jrl_gamma: Option<f64>,
    #[arg(long = "sinkhorn-epsilon", value_name = "EPS")]
    sinkhorn_epsilon: Option<f64>,
    #[arg(long = "sinkhorn-iterations", value_name = "N")]
    sinkhorn_iterations: Option<usize>,
    #[arg(long = "control-grid", value_name = "N")]
    control_grid: Option<usize>,
    #[arg(long = "control-cost", value_name = "COST")]
    control_cost: Option<f64>,

    #[arg(long = "attack-max-tokens", value_name = "N")]
    attack_max_tokens: Option<u32>,
    #[arg(long = "target-max-tokens", value_name = "N")]
    target_max_tokens: Option<u32>,
    #[arg(long = "judge-max-tokens", value_name = "N")]
    judge_max_tokens: Option<u32>,
    #[arg(long = "attack-temperature", value_name = "T")]
    attack_temperature: Option<f64>,
    #[arg(long = "target-temperature", value_name = "T")]
    target_temperature: Option<f64>,
    #[arg(long = "judge-temperature", value_name = "T")]
    judge_temperature: Option<f64>,
    #[arg(long = "attack-top-p", value_name = "P")]
    attack_top_p: Option<f64>,
    #[arg(long = "target-top-p", value_name = "P")]
    target_top_p: Option<f64>,
    #[arg(long = "judge-top-p", value_name = "P")]
    judge_top_p: Option<f64>,

    /// Write the full RedTeamRun JSON to this path.
    #[arg(long = "output", short = 'o', value_name = "PATH")]
    output: Option<PathBuf>,
    /// Print full RedTeamRun JSON to stdout.
    #[arg(long = "json")]
    json: bool,
}

pub(super) async fn run_redteam_subcommand(sub: RedTeamSubcommand) -> anyhow::Result<()> {
    match sub {
        RedTeamSubcommand::Run(args) => run_redteam(args).await,
    }
}

async fn run_redteam(args: RedTeamRunArgs) -> anyhow::Result<()> {
    let cfg = jfc_config::load().redteam.unwrap_or_default();
    if !args.allow_red_team && !cfg.enabled {
        anyhow::bail!(
            "redteam methods are controlled evaluators. Re-run with --allow-red-team \
             or set [redteam] enabled = true in config.toml."
        );
    }

    let objective = objective_text(&args)?;
    if objective.trim().is_empty() {
        anyhow::bail!("redteam objective is empty");
    }
    let method = args
        .method
        .map(RedTeamMethod::from)
        .or_else(|| cfg.method.as_deref().and_then(parse_method))
        .unwrap_or(RedTeamMethod::Tap);

    let init = build_providers();
    let default_model = init.model.as_str();
    let attacker_model = pick_string(args.attacker_model.as_ref(), cfg.attacker_model.as_ref())
        .unwrap_or(default_model);
    let target_model =
        pick_string(args.target_model.as_ref(), cfg.target_model.as_ref()).unwrap_or(default_model);
    let judge_model =
        pick_string(args.judge_model.as_ref(), cfg.judge_model.as_ref()).unwrap_or(default_model);

    let (attacker_provider, attacker_model) = resolve_role(
        &init.providers,
        init.active_idx,
        default_model,
        attacker_model,
        pick_string(
            args.attacker_provider.as_ref(),
            cfg.attacker_provider.as_ref(),
        ),
    )?;
    let (target_provider, target_model) = resolve_role(
        &init.providers,
        init.active_idx,
        default_model,
        target_model,
        pick_string(args.target_provider.as_ref(), cfg.target_provider.as_ref()),
    )?;

    let attacker = ProviderRewriteModel::new(
        "attacker",
        attacker_provider,
        attacker_model,
        role_options(
            args.attack_max_tokens,
            cfg.attack_max_tokens,
            500,
            args.attack_temperature,
            cfg.attack_temperature,
            Some(1.0),
            args.attack_top_p,
            cfg.attack_top_p,
            Some(0.9),
        ),
    );
    let target = ProviderRewriteModel::new(
        "target",
        target_provider,
        target_model,
        role_options(
            args.target_max_tokens,
            cfg.target_max_tokens,
            256,
            args.target_temperature,
            cfg.target_temperature,
            None,
            args.target_top_p,
            cfg.target_top_p,
            None,
        ),
    );

    let judge_kind = args
        .judge
        .or_else(|| cfg.judge.as_deref().and_then(parse_judge_kind))
        .unwrap_or(JudgeKind::Model);
    let judge_model_box: Box<dyn RewriteModel> = match judge_kind {
        JudgeKind::Model => {
            let (judge_provider, judge_model) = resolve_role(
                &init.providers,
                init.active_idx,
                default_model,
                judge_model,
                pick_string(args.judge_provider.as_ref(), cfg.judge_provider.as_ref()),
            )?;
            Box::new(ProviderRewriteModel::new(
                "judge",
                judge_provider,
                judge_model,
                role_options(
                    args.judge_max_tokens,
                    cfg.judge_max_tokens,
                    256,
                    args.judge_temperature,
                    cfg.judge_temperature,
                    Some(0.0),
                    args.judge_top_p,
                    cfg.judge_top_p,
                    None,
                ),
            ))
        }
        JudgeKind::Heuristic => Box::new(RedTeamHeuristicJudge::default()),
    };

    let mut rt_config = RedTeamConfig::new(objective.clone());
    rt_config.target_start = args.target_start.clone();
    rt_config.max_iterations = pick_usize(args.n_iterations, cfg.n_iterations, 3);
    rt_config.n_streams = pick_usize(args.n_streams, cfg.n_streams, 3);
    rt_config.branch_factor = pick_usize(args.branch_factor, cfg.branch_factor, 3);
    rt_config.prune_width = pick_usize(args.prune_width, cfg.prune_width, 3);
    rt_config.population_size = pick_usize(args.population_size, cfg.population_size, 8);
    rt_config.generations = pick_usize(args.generations, cfg.generations, 3);
    rt_config.max_turns = pick_usize(args.max_turns, cfg.max_turns, 6);
    rt_config.success_threshold = pick_f64(args.success_threshold, cfg.success_threshold, 10.0);
    rt_config.proact_defense = args.proact_defense || cfg.proact_defense.unwrap_or(false);
    rt_config.robot_context = args.robot_context.clone().or(cfg.robot_context.clone());
    rt_config.beta0 = pick_f64(args.beta0, cfg.beta0, 1.0);
    rt_config.casp_drift = pick_f64(args.casp_drift, cfg.casp_drift, 0.2);
    rt_config.embedding_dim = pick_usize(args.embedding_dim, cfg.embedding_dim, 32);
    rt_config.bo_candidates = pick_usize(args.bo_candidates, cfg.bo_candidates, 8);
    rt_config.jrl_learning_rate = pick_f64(args.jrl_learning_rate, cfg.jrl_learning_rate, 0.2);
    rt_config.jrl_gamma = pick_f64(args.jrl_gamma, cfg.jrl_gamma, 0.95);
    rt_config.sinkhorn_epsilon = pick_f64(args.sinkhorn_epsilon, cfg.sinkhorn_epsilon, 0.1);
    rt_config.sinkhorn_iterations =
        pick_usize(args.sinkhorn_iterations, cfg.sinkhorn_iterations, 50);
    rt_config.control_grid = pick_usize(args.control_grid, cfg.control_grid, 21);
    rt_config.control_cost = pick_f64(args.control_cost, cfg.control_cost, 0.4);

    eprintln!(
        "→ redteam method={} attacker={} target={} judge={} threshold={:.1}",
        method.as_str(),
        attacker.describe(),
        target.describe(),
        match judge_kind {
            JudgeKind::Model => "model",
            JudgeKind::Heuristic => "heuristic",
        },
        rt_config.success_threshold,
    );

    let run = RedTeamRunner::new(&attacker, &target, &*judge_model_box)
        .run(method, &rt_config)
        .await?;
    let json = serde_json::to_string_pretty(&run)?;
    if let Some(path) = &args.output {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &json)?;
        eprintln!("wrote {}", path.display());
    }
    if args.json {
        println!("{json}");
    } else {
        print_summary(&run);
    }
    Ok(())
}

fn objective_text(args: &RedTeamRunArgs) -> anyhow::Result<String> {
    match &args.objective {
        Some(objective) => Ok(objective.clone()),
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

fn parse_method(raw: &str) -> Option<RedTeamMethod> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "tap" => Some(RedTeamMethod::Tap),
        "autodan" | "auto_dan" => Some(RedTeamMethod::AutoDan),
        "crescendo" => Some(RedTeamMethod::Crescendo),
        "goat" => Some(RedTeamMethod::Goat),
        "robopair" | "robo_pair" => Some(RedTeamMethod::RoboPair),
        "proact" => Some(RedTeamMethod::ProAct),
        "pair-markov" | "pair_markov" | "pairmarkov" => Some(RedTeamMethod::PairMarkov),
        "tap-ucb" | "tap_ucb" | "tapucb" => Some(RedTeamMethod::TapUcb),
        "boja" => Some(RedTeamMethod::Boja),
        "casp" => Some(RedTeamMethod::Casp),
        "j-rl" | "j_rl" | "jrl" => Some(RedTeamMethod::Jrl),
        "pot-j" | "pot_j" | "potj" => Some(RedTeamMethod::PotJ),
        "soc-prompt" | "soc_prompt" | "socprompt" => Some(RedTeamMethod::SocPrompt),
        _ => None,
    }
}

fn parse_judge_kind(raw: &str) -> Option<JudgeKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "model" | "llm" => Some(JudgeKind::Model),
        "heuristic" | "local" => Some(JudgeKind::Heuristic),
        _ => None,
    }
}

fn pick_string<'a>(cli: Option<&'a String>, cfg: Option<&'a String>) -> Option<&'a str> {
    cli.or(cfg).map(String::as_str).filter(|s| !s.is_empty())
}

fn pick_usize(cli: Option<usize>, cfg: Option<usize>, default: usize) -> usize {
    cli.or(cfg).unwrap_or(default)
}

fn pick_f64(cli: Option<f64>, cfg: Option<f64>, default: f64) -> f64 {
    cli.or(cfg).unwrap_or(default)
}

fn role_options(
    cli_max_tokens: Option<u32>,
    cfg_max_tokens: Option<u32>,
    default_max_tokens: u32,
    cli_temperature: Option<f64>,
    cfg_temperature: Option<f64>,
    default_temperature: Option<f64>,
    cli_top_p: Option<f64>,
    cfg_top_p: Option<f64>,
    default_top_p: Option<f64>,
) -> RoleOptions {
    RoleOptions {
        max_tokens: cli_max_tokens
            .or(cfg_max_tokens)
            .unwrap_or(default_max_tokens),
        temperature: cli_temperature.or(cfg_temperature).or(default_temperature),
        top_p: cli_top_p.or(cfg_top_p).or(default_top_p),
    }
}

fn resolve_role(
    providers: &[Arc<dyn Provider>],
    active_idx: usize,
    default_model: &str,
    model_arg: &str,
    provider_arg: Option<&str>,
) -> anyhow::Result<(Arc<dyn Provider>, String)> {
    if let Some(name) = provider_arg {
        let provider = providers
            .iter()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no provider named `{name}`"))?;
        let bare = model_arg
            .split_once('/')
            .map(|(_, model)| model)
            .unwrap_or(model_arg);
        return Ok((provider, bare.to_string()));
    }
    if let Some(resolution) = resolve_provider_model(providers, model_arg) {
        return Ok((resolution.provider, resolution.model.as_str().to_string()));
    }
    let provider = providers
        .get(active_idx)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no providers configured"))?;
    Ok((provider, default_model.to_string()))
}

#[derive(Debug, Clone, Copy)]
struct RoleOptions {
    max_tokens: u32,
    temperature: Option<f64>,
    top_p: Option<f64>,
}

struct ProviderRewriteModel {
    role: &'static str,
    provider: Arc<dyn Provider>,
    model: String,
    options: RoleOptions,
}

impl ProviderRewriteModel {
    fn new(
        role: &'static str,
        provider: Arc<dyn Provider>,
        model: String,
        options: RoleOptions,
    ) -> Self {
        Self {
            role,
            provider,
            model,
            options,
        }
    }

    fn describe(&self) -> String {
        format!("{}/{}", self.provider.name(), self.model)
    }
}

#[async_trait]
impl RewriteModel for ProviderRewriteModel {
    async fn complete(&self, system: &str, user: &str) -> jfc_audit::Result<String> {
        self.provider
            .ensure_auth()
            .await
            .map_err(|err| AuditError::Internal {
                message: format!("redteam {} auth failed: {err}", self.role),
            })?;
        let mut opts = StreamOptions::new(self.model.clone())
            .max_tokens(self.options.max_tokens)
            .system(system.to_string());
        if let Some(t) = self.options.temperature {
            opts = opts.temperature(t);
        }
        if let Some(p) = self.options.top_p {
            opts = opts.top_p(p);
        }
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(user.to_string())],
        }];
        let response = self
            .provider
            .complete(messages, &opts)
            .await
            .map_err(|err| AuditError::Internal {
                message: format!("redteam {} completion failed: {err}", self.role),
            })?;
        Ok(response.content)
    }
}

fn print_summary(run: &RedTeamRun) {
    println!(
        "{} {} after {} attempt(s); best_score={:.1}",
        run.method.as_str(),
        if run.succeeded {
            "succeeded"
        } else {
            "exhausted budget"
        },
        run.attempts.len(),
        run.best_score
    );
    println!("formalism: {}", run.formalism.problem);
    if let Some(bound) = &run.formalism.query_bound {
        println!("query_bound: {bound}");
    }
    println!("convergence: {}", run.formalism.convergence_proof);
    if let Some(best) = run.best_attempt() {
        println!(
            "\nBest: iteration={} turn={} strategy={} verdict={} score={:.1}",
            best.iteration,
            best.turn,
            best.strategy,
            best.judgment.verdict.as_str(),
            best.judgment.score
        );
        println!("\nPrompt:\n{}", best.candidate_prompt);
        println!("\nTarget response:\n{}", best.target_response);
        if !best.judgment.rationale.is_empty() {
            println!("\nJudge rationale:\n{}", best.judgment.rationale);
        }
    }
}

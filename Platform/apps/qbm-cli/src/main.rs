//! Q-BenchMed Platform command-line interface.

use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use qbm_app::{AppError, PlatformApp};
use qbm_domain::{Decision, GraphPolicy, IntakePolicy, InventoryId, RunId, RunMode};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "qbm",
    version,
    about = "Quantum-classical benchmarking for biomedical input optimization"
)]
struct Cli {
    /// Platform state directory. Defaults to `QBM_DATA_DIR` or `.qbenchmed`.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Emit compact JSON instead of indented JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the loopback-only browser interface.
    Serve(ServeArgs),
    /// Initialize and inspect the local platform store.
    Init,
    /// Register and inspect projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Start, inspect and approve audit runs.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    /// Record material stage outputs.
    Stage {
        #[command(subcommand)]
        command: StageCommand,
    },
    /// Store and inspect content-addressed artifacts.
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
    },
    /// Scan project sources into deterministic, reviewable inventories.
    Intake {
        #[command(subcommand)]
        command: IntakeCommand,
    },
    /// Create and inspect immutable source snapshots.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    /// Build and inspect scanner-neutral raw project graphs.
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    /// Run and inspect generic structural audits.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// Install, detect and select declarative domain adapters.
    Adapter {
        #[command(subcommand)]
        command: AdapterCommand,
    },
    /// Build and inspect adapter-projected semantic graphs and audits.
    Semantic {
        #[command(subcommand)]
        command: SemanticCommand,
    },
    /// Resolve and inspect adapter-defined semantic views.
    Projection {
        #[command(subcommand)]
        command: ProjectionCommand,
    },
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Local loopback port. Use 0 to let the operating system choose one.
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Print the URL without opening the default browser.
    #[arg(long)]
    no_open: bool,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Register an existing source directory or supported archive read-only.
    Add(ProjectAdd),
    /// List projects.
    List,
    /// Show one project.
    Show {
        /// Project ID.
        project_id: String,
    },
}

#[derive(Debug, Args)]
struct ProjectAdd {
    /// Stable project ID.
    project_id: String,
    /// User-facing project name.
    #[arg(long)]
    name: String,
    /// Existing project source directory or .zip/.tar/.tar.gz/.tgz archive.
    #[arg(long)]
    source: PathBuf,
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    /// Start a new approval-gated analysis run.
    Start {
        /// Registered project ID.
        project_id: String,
        /// Approval/audit profile.
        #[arg(long, value_enum, default_value_t = CliRunMode::Standard)]
        mode: CliRunMode,
    },
    /// List runs.
    List {
        /// Restrict results to one project.
        #[arg(long)]
        project: Option<String>,
    },
    /// Show one run.
    Show {
        /// Run UUID.
        run_id: String,
    },
    /// Approve the exact current output of a stage.
    Approve(DecisionArgs),
    /// Reject the exact current output of a stage.
    Reject(DecisionArgs),
    /// Request a revised output for a stage.
    RequestChanges(DecisionArgs),
    /// List approval decisions.
    Approvals {
        /// Run UUID.
        run_id: String,
    },
    /// List append-only run events.
    Events {
        /// Run UUID.
        run_id: String,
    },
    /// Verify a run's event hash chain.
    VerifyEvents {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Args)]
struct DecisionArgs {
    /// Run UUID.
    run_id: String,
    /// Material stage ID.
    #[arg(long)]
    stage: String,
    /// Exact stage-output SHA-256 reviewed by the actor.
    #[arg(long)]
    expected_hash: String,
    /// Portable actor/reviewer ID.
    #[arg(long)]
    actor: String,
    /// Explanation. Required for reject and request-changes.
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Subcommand)]
enum StageCommand {
    /// Record an output and place the stage into waiting-approval state.
    Complete {
        /// Run UUID.
        run_id: String,
        /// Material stage ID.
        stage_id: String,
        /// SHA-256 of the exact stage output.
        #[arg(long)]
        output_hash: String,
    },
}

#[derive(Debug, Subcommand)]
enum ArtifactCommand {
    /// Copy one file into the content-addressed local store.
    Put {
        /// Local file.
        source: PathBuf,
        /// Optional declared media type.
        #[arg(long)]
        media_type: Option<String>,
    },
    /// Show artifact metadata by SHA-256.
    Show {
        /// Artifact SHA-256.
        sha256: String,
    },
}

#[derive(Debug, Subcommand)]
enum IntakeCommand {
    /// Read, classify, hash and freeze a run's registered source.
    Scan(IntakeScan),
    /// Show one inventory manifest.
    Show {
        /// Inventory UUID.
        inventory_id: String,
    },
    /// List inventories for a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Args)]
struct IntakeScan {
    /// Run UUID.
    run_id: String,
    /// Add a name excluded at any depth. May be repeated.
    #[arg(long)]
    exclude_name: Vec<String>,
    /// Add an excluded relative path prefix. May be repeated.
    #[arg(long)]
    exclude_prefix: Vec<String>,
    /// Restrict intake to a relative path prefix. May be repeated.
    #[arg(long)]
    include_prefix: Vec<String>,
    /// Override the maximum number of observed entries.
    #[arg(long)]
    max_entries: Option<u64>,
    /// Override the maximum total included bytes.
    #[arg(long)]
    max_total_bytes: Option<u64>,
    /// Override the maximum size of one included file.
    #[arg(long)]
    max_single_file_bytes: Option<u64>,
    /// Override the maximum path nesting depth.
    #[arg(long)]
    max_depth: Option<u32>,
    /// Override the maximum archive expansion ratio.
    #[arg(long)]
    max_compression_ratio: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum SnapshotCommand {
    /// Create a snapshot after the exact inventory stage is approved.
    Create {
        /// Run UUID.
        run_id: String,
        /// Approved inventory UUID.
        inventory_id: String,
    },
    /// Show one snapshot by SHA-256 identity.
    Show {
        /// Snapshot SHA-256.
        snapshot_id: String,
    },
    /// List snapshots attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum GraphCommand {
    /// Build a graph after the exact source-snapshot stage is approved.
    Build(GraphBuild),
    /// Show one graph by SHA-256 identity.
    Show {
        /// Graph SHA-256.
        graph_id: String,
    },
    /// List graphs attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Args)]
struct GraphBuild {
    /// Run UUID.
    run_id: String,
    /// Approved snapshot SHA-256.
    snapshot_id: String,
    /// Add a structured-document identifier key. May be repeated.
    #[arg(long)]
    identifier_key: Vec<String>,
    /// Add an explicit structured-document reference key. May be repeated.
    #[arg(long)]
    reference_key: Vec<String>,
    /// Add a key whose children are definitions. May be repeated.
    #[arg(long)]
    definition_container: Vec<String>,
    /// Override maximum bytes parsed from one supported source file.
    #[arg(long)]
    max_parse_file_bytes: Option<u64>,
    /// Override maximum graph nodes.
    #[arg(long)]
    max_nodes: Option<u64>,
    /// Override maximum graph edges.
    #[arg(long)]
    max_edges: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Run generic policies after the exact raw-graph stage is approved.
    Run {
        /// Run UUID.
        run_id: String,
        /// Approved graph SHA-256.
        graph_id: String,
    },
    /// Show one report by SHA-256 identity.
    Show {
        /// Report SHA-256.
        report_id: String,
    },
    /// List reports attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterCommand {
    /// Manage immutable declarative adapter packages.
    Package {
        #[command(subcommand)]
        command: AdapterPackageCommand,
    },
    /// Inspect digest-specific adapter conformance certificates.
    Conformance {
        #[command(subcommand)]
        command: AdapterConformanceCommand,
    },
    /// Detect installed adapters against an approved raw graph.
    Detection {
        #[command(subcommand)]
        command: AdapterDetectionCommand,
    },
    /// Lock exact detected adapter package hashes into an approved plan.
    Plan {
        #[command(subcommand)]
        command: AdapterPlanCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterPackageCommand {
    /// Validate a package and persist its pass/fail conformance report without installing it.
    Check {
        /// Adapter package JSON file.
        source: PathBuf,
    },
    /// Validate and install a data-only JSON adapter package.
    Add {
        /// Adapter package JSON file.
        source: PathBuf,
    },
    /// List installed immutable adapter packages.
    List,
    /// Show one package by canonical SHA-256.
    Show {
        /// Installed package SHA-256.
        package_hash: String,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterConformanceCommand {
    /// Show one conformance certificate by SHA-256.
    Show {
        /// Conformance report SHA-256.
        report_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterDetectionCommand {
    /// Evaluate installed adapters against an exact approved raw graph.
    Run {
        /// Run UUID.
        run_id: String,
        /// Approved raw graph SHA-256.
        graph_id: String,
    },
    /// Show one detection report by SHA-256.
    Show {
        /// Detection report SHA-256.
        detection_id: String,
    },
    /// List detection reports attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterPlanCommand {
    /// Create an immutable plan from an approved detection report.
    Create {
        /// Run UUID.
        run_id: String,
        /// Approved adapter-detection report SHA-256.
        detection_id: String,
        /// Exact installed package SHA-256. May be repeated.
        #[arg(long = "package-hash", required = true)]
        package_hashes: Vec<String>,
    },
    /// Show one adapter plan by SHA-256.
    Show {
        /// Adapter plan SHA-256.
        plan_id: String,
    },
    /// List adapter plans attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SemanticCommand {
    /// Build and inspect semantic graphs.
    Graph {
        #[command(subcommand)]
        command: SemanticGraphCommand,
    },
    /// Run and inspect adapter policy audits.
    Audit {
        #[command(subcommand)]
        command: SemanticAuditCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SemanticGraphCommand {
    /// Project an approved adapter plan into a semantic graph.
    Build {
        /// Run UUID.
        run_id: String,
        /// Approved adapter plan SHA-256.
        plan_id: String,
    },
    /// Show one semantic graph by SHA-256.
    Show {
        /// Semantic graph SHA-256.
        graph_id: String,
    },
    /// List semantic graphs attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SemanticAuditCommand {
    /// Evaluate adapter policy packs against an approved semantic graph.
    Run {
        /// Run UUID.
        run_id: String,
        /// Approved semantic graph SHA-256.
        graph_id: String,
    },
    /// Show one semantic audit report by SHA-256.
    Show {
        /// Semantic audit report SHA-256.
        report_id: String,
    },
    /// List semantic audit reports attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectionCommand {
    /// Resolve and inspect adapter-declared projection catalogs.
    Catalog {
        #[command(subcommand)]
        command: ProjectionCatalogCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectionCatalogCommand {
    /// Resolve projections from matching approved semantic graph and audit outputs.
    Build {
        /// Run UUID.
        run_id: String,
        /// Approved semantic graph SHA-256.
        graph_id: String,
        /// Approved semantic audit report SHA-256.
        report_id: String,
    },
    /// Show one projection catalog by SHA-256.
    Show {
        /// Projection catalog SHA-256.
        catalog_id: String,
    },
    /// List projection catalogs attached to a run.
    List {
        /// Run UUID.
        run_id: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliRunMode {
    Onboarding,
    Quick,
    Standard,
    Deep,
    Governed,
    Continuous,
}

impl From<CliRunMode> for RunMode {
    fn from(value: CliRunMode) -> Self {
        match value {
            CliRunMode::Onboarding => Self::Onboarding,
            CliRunMode::Quick => Self::Quick,
            CliRunMode::Standard => Self::Standard,
            CliRunMode::Deep => Self::Deep,
            CliRunMode::Governed => Self::Governed,
            CliRunMode::Continuous => Self::Continuous,
        }
    }
}

#[derive(Debug, Serialize)]
struct InitOutput<'a> {
    status: &'static str,
    data_directory: &'a Path,
}

#[derive(Debug, Serialize)]
struct VerifyOutput {
    run_id: RunId,
    event_chain: &'static str,
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    App(#[from] AppError),
    #[error(transparent)]
    Web(#[from] qbm_web::WebServerError),
    #[error("cannot start the asynchronous browser service: {0}")]
    Runtime(#[from] std::io::Error),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn execute(cli: Cli) -> Result<(), CliError> {
    let data_directory = cli
        .data_dir
        .or_else(|| env::var_os("QBM_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".qbenchmed"));
    let app = PlatformApp::open(&data_directory)?;

    match cli.command {
        Command::Serve(arguments) => {
            let config = qbm_web::WebConfig {
                open_browser: !arguments.no_open,
                ..qbm_web::WebConfig::default()
            };
            drop(app);
            tokio::runtime::Runtime::new()?.block_on(qbm_web::serve(
                &data_directory,
                arguments.port,
                config,
            ))?;
        }
        Command::Init => emit(
            &InitOutput {
                status: "initialized",
                data_directory: app.data_directory(),
            },
            cli.json,
        ),
        Command::Project { command } => match command {
            ProjectCommand::Add(arguments) => emit(
                &app.register_project(&arguments.project_id, &arguments.name, &arguments.source)?,
                cli.json,
            ),
            ProjectCommand::List => emit(&app.projects()?, cli.json),
            ProjectCommand::Show { project_id } => emit(&app.project(&project_id)?, cli.json),
        },
        Command::Run { command } => match command {
            RunCommand::Start { project_id, mode } => {
                emit(&app.start_run(&project_id, mode.into())?, cli.json);
            }
            RunCommand::List { project } => emit(&app.runs(project.as_deref())?, cli.json),
            RunCommand::Show { run_id } => emit(&app.run(parse_run_id(&run_id)?)?, cli.json),
            RunCommand::Approve(arguments) => {
                emit(&decide(&app, arguments, Decision::Approve)?, cli.json);
            }
            RunCommand::Reject(arguments) => {
                emit(&decide(&app, arguments, Decision::Reject)?, cli.json);
            }
            RunCommand::RequestChanges(arguments) => emit(
                &decide(&app, arguments, Decision::RequestChanges)?,
                cli.json,
            ),
            RunCommand::Approvals { run_id } => {
                emit(&app.approvals(parse_run_id(&run_id)?)?, cli.json);
            }
            RunCommand::Events { run_id } => emit(&app.events(parse_run_id(&run_id)?)?, cli.json),
            RunCommand::VerifyEvents { run_id } => {
                let run_id = parse_run_id(&run_id)?;
                app.verify_events(run_id)?;
                emit(
                    &VerifyOutput {
                        run_id,
                        event_chain: "valid",
                    },
                    cli.json,
                );
            }
        },
        Command::Stage { command } => match command {
            StageCommand::Complete {
                run_id,
                stage_id,
                output_hash,
            } => emit(
                &app.complete_stage(parse_run_id(&run_id)?, &stage_id, &output_hash)?,
                cli.json,
            ),
        },
        Command::Artifact { command } => match command {
            ArtifactCommand::Put { source, media_type } => {
                emit(&app.put_artifact(&source, media_type)?, cli.json);
            }
            ArtifactCommand::Show { sha256 } => emit(&app.artifact(&sha256)?, cli.json),
        },
        Command::Intake { command } => match command {
            IntakeCommand::Scan(arguments) => {
                let run_id = parse_run_id(&arguments.run_id)?;
                let policy = intake_policy(arguments);
                emit(&app.scan_intake(run_id, &policy)?, cli.json);
            }
            IntakeCommand::Show { inventory_id } => {
                emit(
                    &app.inventory(parse_inventory_id(&inventory_id)?)?,
                    cli.json,
                );
            }
            IntakeCommand::List { run_id } => {
                emit(&app.inventories(parse_run_id(&run_id)?)?, cli.json);
            }
        },
        Command::Snapshot { command } => match command {
            SnapshotCommand::Create {
                run_id,
                inventory_id,
            } => emit(
                &app.create_snapshot(parse_run_id(&run_id)?, parse_inventory_id(&inventory_id)?)?,
                cli.json,
            ),
            SnapshotCommand::Show { snapshot_id } => {
                emit(&app.snapshot(&snapshot_id)?, cli.json);
            }
            SnapshotCommand::List { run_id } => {
                emit(&app.snapshots(parse_run_id(&run_id)?)?, cli.json);
            }
        },
        Command::Graph { command } => match command {
            GraphCommand::Build(arguments) => {
                let run_id = parse_run_id(&arguments.run_id)?;
                let snapshot_id = arguments.snapshot_id.clone();
                let policy = graph_policy(arguments);
                emit(&app.build_graph(run_id, &snapshot_id, &policy)?, cli.json);
            }
            GraphCommand::Show { graph_id } => emit(&app.graph(&graph_id)?, cli.json),
            GraphCommand::List { run_id } => {
                emit(&app.graphs(parse_run_id(&run_id)?)?, cli.json);
            }
        },
        Command::Audit { command } => match command {
            AuditCommand::Run { run_id, graph_id } => emit(
                &app.run_generic_audit(parse_run_id(&run_id)?, &graph_id)?,
                cli.json,
            ),
            AuditCommand::Show { report_id } => emit(&app.report(&report_id)?, cli.json),
            AuditCommand::List { run_id } => {
                emit(&app.reports(parse_run_id(&run_id)?)?, cli.json);
            }
        },
        Command::Adapter { command } => match command {
            AdapterCommand::Package { command } => match command {
                AdapterPackageCommand::Check { source } => {
                    emit(&app.check_adapter(&source)?, cli.json);
                }
                AdapterPackageCommand::Add { source } => {
                    emit(&app.install_adapter(&source)?, cli.json);
                }
                AdapterPackageCommand::List => emit(&app.adapters()?, cli.json),
                AdapterPackageCommand::Show { package_hash } => {
                    emit(&app.adapter(&package_hash)?, cli.json);
                }
            },
            AdapterCommand::Conformance { command } => match command {
                AdapterConformanceCommand::Show { report_id } => {
                    emit(&app.adapter_conformance(&report_id)?, cli.json);
                }
            },
            AdapterCommand::Detection { command } => match command {
                AdapterDetectionCommand::Run { run_id, graph_id } => emit(
                    &app.detect_adapters(parse_run_id(&run_id)?, &graph_id)?,
                    cli.json,
                ),
                AdapterDetectionCommand::Show { detection_id } => {
                    emit(&app.adapter_detection(&detection_id)?, cli.json);
                }
                AdapterDetectionCommand::List { run_id } => {
                    emit(&app.adapter_detections(parse_run_id(&run_id)?)?, cli.json);
                }
            },
            AdapterCommand::Plan { command } => match command {
                AdapterPlanCommand::Create {
                    run_id,
                    detection_id,
                    package_hashes,
                } => emit(
                    &app.create_adapter_plan(
                        parse_run_id(&run_id)?,
                        &detection_id,
                        &package_hashes,
                    )?,
                    cli.json,
                ),
                AdapterPlanCommand::Show { plan_id } => {
                    emit(&app.adapter_plan(&plan_id)?, cli.json);
                }
                AdapterPlanCommand::List { run_id } => {
                    emit(&app.adapter_plans(parse_run_id(&run_id)?)?, cli.json);
                }
            },
        },
        Command::Semantic { command } => match command {
            SemanticCommand::Graph { command } => match command {
                SemanticGraphCommand::Build { run_id, plan_id } => emit(
                    &app.build_semantic_graph(parse_run_id(&run_id)?, &plan_id)?,
                    cli.json,
                ),
                SemanticGraphCommand::Show { graph_id } => {
                    emit(&app.semantic_graph(&graph_id)?, cli.json);
                }
                SemanticGraphCommand::List { run_id } => {
                    emit(&app.semantic_graphs(parse_run_id(&run_id)?)?, cli.json);
                }
            },
            SemanticCommand::Audit { command } => match command {
                SemanticAuditCommand::Run { run_id, graph_id } => emit(
                    &app.run_semantic_audit(parse_run_id(&run_id)?, &graph_id)?,
                    cli.json,
                ),
                SemanticAuditCommand::Show { report_id } => {
                    emit(&app.semantic_report(&report_id)?, cli.json);
                }
                SemanticAuditCommand::List { run_id } => {
                    emit(&app.semantic_reports(parse_run_id(&run_id)?)?, cli.json);
                }
            },
        },
        Command::Projection { command } => match command {
            ProjectionCommand::Catalog { command } => match command {
                ProjectionCatalogCommand::Build {
                    run_id,
                    graph_id,
                    report_id,
                } => emit(
                    &app.build_projection_catalog(parse_run_id(&run_id)?, &graph_id, &report_id)?,
                    cli.json,
                ),
                ProjectionCatalogCommand::Show { catalog_id } => {
                    emit(&app.projection_catalog(&catalog_id)?, cli.json);
                }
                ProjectionCatalogCommand::List { run_id } => {
                    emit(&app.projection_catalogs(parse_run_id(&run_id)?)?, cli.json);
                }
            },
        },
    }
    Ok(())
}

fn decide(
    app: &PlatformApp,
    arguments: DecisionArgs,
    decision: Decision,
) -> Result<qbm_domain::Approval, AppError> {
    app.decide_stage(
        parse_run_id(&arguments.run_id)?,
        &arguments.stage,
        &arguments.expected_hash,
        decision,
        &arguments.actor,
        arguments.reason,
    )
}

fn parse_run_id(value: &str) -> Result<RunId, AppError> {
    RunId::from_str(value).map_err(|error| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid run UUID {value:?}: {error}"),
        ))
    })
}

fn parse_inventory_id(value: &str) -> Result<InventoryId, AppError> {
    InventoryId::from_str(value).map_err(|error| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid inventory UUID {value:?}: {error}"),
        ))
    })
}

fn intake_policy(arguments: IntakeScan) -> IntakePolicy {
    let mut policy = IntakePolicy::default();
    policy.excluded_names.extend(arguments.exclude_name);
    policy.excluded_prefixes.extend(arguments.exclude_prefix);
    policy.included_prefixes.extend(arguments.include_prefix);
    if let Some(value) = arguments.max_entries {
        policy.max_entries = value;
    }
    if let Some(value) = arguments.max_total_bytes {
        policy.max_total_bytes = value;
    }
    if let Some(value) = arguments.max_single_file_bytes {
        policy.max_single_file_bytes = value;
    }
    if let Some(value) = arguments.max_depth {
        policy.max_depth = value;
    }
    if let Some(value) = arguments.max_compression_ratio {
        policy.max_compression_ratio = value;
    }
    policy
}

fn graph_policy(arguments: GraphBuild) -> GraphPolicy {
    let mut policy = GraphPolicy::default();
    policy.identifier_keys.extend(arguments.identifier_key);
    policy.reference_keys.extend(arguments.reference_key);
    policy
        .definition_container_keys
        .extend(arguments.definition_container);
    if let Some(value) = arguments.max_parse_file_bytes {
        policy.max_parse_file_bytes = value;
    }
    if let Some(value) = arguments.max_nodes {
        policy.max_nodes = value;
    }
    if let Some(value) = arguments.max_edges {
        policy.max_edges = value;
    }
    policy
}

fn emit(value: &impl Serialize, compact: bool) {
    let result = if compact {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    };
    match result {
        Ok(output) => println!("{output}"),
        Err(error) => eprintln!("error: cannot serialize command output: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_and_semantic_workflows_have_stable_cli_shapes() {
        let serve = Cli::try_parse_from(["qbm", "serve", "--port", "0", "--no-open"]);
        assert!(serve.is_ok());

        let package = Cli::try_parse_from(["qbm", "adapter", "package", "add", "adapter.json"]);
        assert!(package.is_ok());

        let plan = Cli::try_parse_from([
            "qbm",
            "adapter",
            "plan",
            "create",
            "00000000-0000-4000-8000-000000000000",
            &"a".repeat(64),
            "--package-hash",
            &"b".repeat(64),
        ]);
        assert!(plan.is_ok());

        let semantic = Cli::try_parse_from([
            "qbm",
            "semantic",
            "graph",
            "build",
            "00000000-0000-4000-8000-000000000000",
            &"c".repeat(64),
        ]);
        assert!(semantic.is_ok());
    }

    #[test]
    fn adapter_plan_rejects_an_empty_package_selection() {
        let result = Cli::try_parse_from([
            "qbm",
            "adapter",
            "plan",
            "create",
            "00000000-0000-4000-8000-000000000000",
            &"a".repeat(64),
        ]);
        assert!(result.is_err());
    }
}

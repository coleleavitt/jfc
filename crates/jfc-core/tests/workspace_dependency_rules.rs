#[path = "workspace_dependency_rules/engine_root.rs"]
mod engine_root;
#[path = "workspace_dependency_rules/session_ownership.rs"]
mod session_ownership;
#[path = "workspace_dependency_rules/support.rs"]
mod support;

use std::collections::BTreeSet;

use support::{
    ForbiddenDependencyEdge, WorkspaceDependencies, WorkspacePackageRoots,
    assert_no_forbidden_edges, forbidden_edges_except, read_workspace_dependencies,
    read_workspace_package_roots,
};

const PACKAGE_ROOT_POLICY: &str = "Target Short Package Root Freeze";

#[derive(Debug, Clone, Copy)]
struct TargetPackageRoot {
    path: &'static str,
    owner: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct TemporaryPackageRoot {
    package: &'static str,
    path: &'static str,
    target_path: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForbiddenPackageRoot {
    package: String,
    path: String,
    rule: &'static str,
}

const CORE_ALLOWED_WORKSPACE_DEPS: &[&str] = &[];
const SDK_ALLOWED_WORKSPACE_DEPS: &[&str] = &["jfc-core"];
const HOST_ALLOWED_WORKSPACE_DEPS: &[&str] = &["jfc-core", "jfc-plugin-sdk"];
const TARGET_SHORT_PACKAGE_ROOTS: &[TargetPackageRoot] = &[
    TargetPackageRoot {
        path: "crates/kernel",
        owner: "minimal runnable kernel",
    },
    TargetPackageRoot {
        path: "crates/protocol",
        owner: "stable DTOs and typed IDs",
    },
    TargetPackageRoot {
        path: "crates/runtime",
        owner: "session runtime factory and service graph",
    },
    TargetPackageRoot {
        path: "crates/session",
        owner: "typed append-entry log and projections",
    },
    TargetPackageRoot {
        path: "crates/plugin",
        owner: "public SDK, host, and extension runner",
    },
    TargetPackageRoot {
        path: "crates/context",
        owner: "context, memory, history, reduction, and health",
    },
    TargetPackageRoot {
        path: "crates/policy",
        owner: "permissions, trust, and safe mode",
    },
    TargetPackageRoot {
        path: "crates/tools",
        owner: "descriptor-backed tool registry and built-in packs",
    },
    TargetPackageRoot {
        path: "crates/providers",
        owner: "provider registry and built-in provider packs",
    },
    TargetPackageRoot {
        path: "crates/orchestration",
        owner: "agents, swarm, council, workflows, and goals",
    },
    TargetPackageRoot {
        path: "crates/daemon",
        owner: "scheduled and detached execution",
    },
    TargetPackageRoot {
        path: "crates/ui-model",
        owner: "frontend-neutral view models",
    },
    TargetPackageRoot {
        path: "crates/tui",
        owner: "ratatui frontend shell",
    },
    TargetPackageRoot {
        path: "crates/cli",
        owner: "command-line frontend shell",
    },
];
const TEMPORARY_PACKAGE_ROOT_ALLOWLIST: &[TemporaryPackageRoot] = &[
    TemporaryPackageRoot {
        package: "jfc",
        path: "crates/jfc",
        target_path: "crates/tui",
    },
    TemporaryPackageRoot {
        package: "jfc-agent",
        path: "crates/jfc-agent",
        target_path: "crates/orchestration",
    },
    TemporaryPackageRoot {
        package: "jfc-agents",
        path: "crates/jfc-agents",
        target_path: "crates/orchestration",
    },
    TemporaryPackageRoot {
        package: "jfc-anthropic-sdk",
        path: "crates/jfc-anthropic-sdk",
        target_path: "crates/providers",
    },
    TemporaryPackageRoot {
        package: "jfc-audit",
        path: "crates/jfc-audit",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-auth",
        path: "crates/jfc-auth",
        target_path: "crates/providers",
    },
    TemporaryPackageRoot {
        package: "jfc-bridge",
        path: "crates/jfc-bridge",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-changeset",
        path: "crates/jfc-changeset",
        target_path: "crates/tools",
    },
    TemporaryPackageRoot {
        package: "jfc-compress",
        path: "crates/jfc-compress",
        target_path: "crates/context",
    },
    TemporaryPackageRoot {
        package: "jfc-config",
        path: "crates/jfc-config",
        target_path: "crates/policy",
    },
    TemporaryPackageRoot {
        package: "jfc-core",
        path: "crates/jfc-core",
        target_path: "crates/protocol",
    },
    TemporaryPackageRoot {
        package: "jfc-daemon",
        path: "crates/jfc-daemon",
        target_path: "crates/daemon",
    },
    TemporaryPackageRoot {
        package: "jfc-design",
        path: "crates/jfc-design",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-economy",
        path: "crates/jfc-economy",
        target_path: "crates/orchestration",
    },
    TemporaryPackageRoot {
        package: "jfc-engine",
        path: "crates/jfc-engine",
        target_path: "crates/kernel",
    },
    TemporaryPackageRoot {
        package: "jfc-knowledge",
        path: "crates/jfc-knowledge",
        target_path: "crates/context",
    },
    TemporaryPackageRoot {
        package: "jfc-learn",
        path: "crates/jfc-learn",
        target_path: "crates/context",
    },
    TemporaryPackageRoot {
        package: "jfc-markdown",
        path: "crates/jfc-markdown",
        target_path: "crates/ui-model",
    },
    TemporaryPackageRoot {
        package: "jfc-mcp",
        path: "crates/jfc-mcp",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-memory",
        path: "crates/jfc-memory",
        target_path: "crates/context",
    },
    TemporaryPackageRoot {
        package: "jfc-plugin-host",
        path: "crates/jfc-plugin-host",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-plugin-sdk",
        path: "crates/jfc-plugin-sdk",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-provider",
        path: "crates/jfc-provider",
        target_path: "crates/providers",
    },
    TemporaryPackageRoot {
        package: "jfc-providers",
        path: "crates/jfc-providers",
        target_path: "crates/providers",
    },
    TemporaryPackageRoot {
        package: "jfc-remote",
        path: "crates/jfc-remote",
        target_path: "crates/runtime",
    },
    TemporaryPackageRoot {
        package: "jfc-session",
        path: "crates/jfc-session",
        target_path: "crates/session",
    },
    TemporaryPackageRoot {
        package: "jfc-theme",
        path: "crates/jfc-theme",
        target_path: "crates/ui-model",
    },
    TemporaryPackageRoot {
        package: "jfc-tools",
        path: "crates/jfc-tools",
        target_path: "crates/tools",
    },
    TemporaryPackageRoot {
        package: "jfc-voice",
        path: "crates/jfc-voice",
        target_path: "crates/plugin",
    },
    TemporaryPackageRoot {
        package: "jfc-web",
        path: "crates/jfc-web",
        target_path: "crates/tools",
    },
];
const TEMPORARY_ENGINE_WORKSPACE_DEP_ALLOWLIST: &[&str] = &[
    "jfc-agent",
    "jfc-agents",
    "jfc-anthropic-sdk",
    "jfc-audit",
    "jfc-auth",
    "jfc-changeset",
    "jfc-compress",
    "jfc-config",
    "jfc-core",
    "jfc-daemon",
    "jfc-design",
    "jfc-economy",
    "jfc-knowledge",
    "jfc-learn",
    "jfc-mcp",
    "jfc-memory",
    "jfc-plugin-host",
    "jfc-plugin-sdk",
    "jfc-provider",
    "jfc-providers",
    "jfc-remote",
    "jfc-session",
    "jfc-tools",
    "jfc-web",
];

#[test]
fn workspace_dependency_rules_package_roots_match_target_or_temporary_allowlist()
-> Result<(), Box<dyn std::error::Error>> {
    let package_roots = read_workspace_package_roots()?;
    let forbidden_package_roots = forbidden_package_roots(&package_roots);

    print_target_package_roots();
    print_temporary_package_root_allowlist();
    println!(
        "current workspace package root count: {}",
        package_roots.len()
    );

    assert!(
        forbidden_package_roots.is_empty(),
        "{PACKAGE_ROOT_POLICY} rejected unowned package roots: {forbidden_package_roots:#?}"
    );

    Ok(())
}

#[test]
fn workspace_dependency_rules_enforce_bare_kernel_direction()
-> Result<(), Box<dyn std::error::Error>> {
    let dependencies = read_workspace_dependencies()?;

    assert_no_forbidden_edges(&forbidden_edges_except(
        &dependencies,
        "jfc-core",
        CORE_ALLOWED_WORKSPACE_DEPS,
        "jfc-core must remain independent of all workspace crates",
    ));
    assert_no_forbidden_edges(&forbidden_edges_except(
        &dependencies,
        "jfc-plugin-sdk",
        SDK_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
    ));
    assert_no_forbidden_edges(&forbidden_edges_except(
        &dependencies,
        "jfc-engine",
        TEMPORARY_ENGINE_WORKSPACE_DEP_ALLOWLIST,
        "jfc-engine may not gain workspace product-domain dependencies beyond the temporary architecture-reset allowlist",
    ));

    Ok(())
}

#[test]
fn plugin_sdk_depends_only_on_core_when_present() -> Result<(), Box<dyn std::error::Error>> {
    let dependencies = read_workspace_dependencies()?;

    let forbidden_edges = forbidden_edges_except(
        &dependencies,
        "jfc-plugin-sdk",
        SDK_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
    );

    assert_no_forbidden_edges(&forbidden_edges);
    Ok(())
}

#[test]
fn plugin_host_depends_only_on_sdk_and_core_when_present() -> Result<(), Box<dyn std::error::Error>>
{
    let dependencies = read_workspace_dependencies()?;

    let forbidden_edges = forbidden_edges_except(
        &dependencies,
        "jfc-plugin-host",
        HOST_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-host may depend on jfc-plugin-sdk and jfc-core only among workspace crates",
    );

    assert_no_forbidden_edges(&forbidden_edges);
    Ok(())
}

#[test]
fn dependency_rule_helper_rejects_forbidden_sdk_edge() {
    let dependencies = WorkspaceDependencies::from([(
        "jfc-plugin-sdk".to_owned(),
        BTreeSet::from(["jfc-core".to_owned(), "jfc-engine".to_owned()]),
    )]);

    let forbidden_edges = forbidden_edges_except(
        &dependencies,
        "jfc-plugin-sdk",
        SDK_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
    );

    assert_eq!(
        forbidden_edges,
        vec![ForbiddenDependencyEdge {
            from: "jfc-plugin-sdk".to_owned(),
            to: "jfc-engine".to_owned(),
            rule: "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
        }]
    );
}

#[test]
fn dependency_rule_helper_rejects_forbidden_host_edge() {
    let dependencies = WorkspaceDependencies::from([(
        "jfc-plugin-host".to_owned(),
        BTreeSet::from([
            "jfc-core".to_owned(),
            "jfc-plugin-sdk".to_owned(),
            "jfc-engine".to_owned(),
        ]),
    )]);

    let forbidden_edges = forbidden_edges_except(
        &dependencies,
        "jfc-plugin-host",
        HOST_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-host may depend on jfc-plugin-sdk and jfc-core only among workspace crates",
    );

    assert_eq!(
        forbidden_edges,
        vec![ForbiddenDependencyEdge {
            from: "jfc-plugin-host".to_owned(),
            to: "jfc-engine".to_owned(),
            rule: "jfc-plugin-host may depend on jfc-plugin-sdk and jfc-core only among workspace crates",
        }]
    );
}

#[test]
fn workspace_dependency_rules_helper_rejects_synthetic_forbidden_edge() {
    let dependencies = WorkspaceDependencies::from([
        (
            "jfc-core".to_owned(),
            BTreeSet::from(["jfc-session".to_owned()]),
        ),
        (
            "jfc-plugin-sdk".to_owned(),
            BTreeSet::from(["jfc-core".to_owned(), "jfc".to_owned()]),
        ),
        (
            "jfc-engine".to_owned(),
            BTreeSet::from(["jfc-core".to_owned(), "jfc-voice".to_owned()]),
        ),
    ]);

    let forbidden_core_edges = forbidden_edges_except(
        &dependencies,
        "jfc-core",
        CORE_ALLOWED_WORKSPACE_DEPS,
        "jfc-core must remain independent of all workspace crates",
    );
    let forbidden_sdk_edges = forbidden_edges_except(
        &dependencies,
        "jfc-plugin-sdk",
        SDK_ALLOWED_WORKSPACE_DEPS,
        "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
    );
    let forbidden_engine_edges = forbidden_edges_except(
        &dependencies,
        "jfc-engine",
        TEMPORARY_ENGINE_WORKSPACE_DEP_ALLOWLIST,
        "jfc-engine may not gain workspace product-domain dependencies beyond the temporary architecture-reset allowlist",
    );

    assert_eq!(
        forbidden_core_edges,
        vec![ForbiddenDependencyEdge {
            from: "jfc-core".to_owned(),
            to: "jfc-session".to_owned(),
            rule: "jfc-core must remain independent of all workspace crates",
        }]
    );
    assert_eq!(
        forbidden_sdk_edges,
        vec![ForbiddenDependencyEdge {
            from: "jfc-plugin-sdk".to_owned(),
            to: "jfc".to_owned(),
            rule: "jfc-plugin-sdk may depend on jfc-core only among workspace crates",
        }]
    );
    assert_eq!(
        forbidden_engine_edges,
        vec![ForbiddenDependencyEdge {
            from: "jfc-engine".to_owned(),
            to: "jfc-voice".to_owned(),
            rule: "jfc-engine may not gain workspace product-domain dependencies beyond the temporary architecture-reset allowlist",
        }]
    );
}

#[test]
fn workspace_dependency_rules_package_root_helper_rejects_synthetic_forbidden_roots() {
    let package_roots = WorkspacePackageRoots::from([
        ("jfc-engine".to_owned(), "crates/jfc-engine".to_owned()),
        ("jfc-kernel".to_owned(), "crates/kernel".to_owned()),
        (
            "jfc-random-feature".to_owned(),
            "crates/jfc-random-feature".to_owned(),
        ),
        (
            "jfc-session-shadow".to_owned(),
            "crates/jfc-session-shadow".to_owned(),
        ),
    ]);

    let forbidden_roots = forbidden_package_roots(&package_roots);

    assert_eq!(
        forbidden_roots,
        vec![
            ForbiddenPackageRoot {
                package: "jfc-random-feature".to_owned(),
                path: "crates/jfc-random-feature".to_owned(),
                rule: PACKAGE_ROOT_POLICY,
            },
            ForbiddenPackageRoot {
                package: "jfc-session-shadow".to_owned(),
                path: "crates/jfc-session-shadow".to_owned(),
                rule: PACKAGE_ROOT_POLICY,
            },
        ]
    );
}

fn forbidden_package_roots(package_roots: &WorkspacePackageRoots) -> Vec<ForbiddenPackageRoot> {
    let target_paths = TARGET_SHORT_PACKAGE_ROOTS
        .iter()
        .map(|root| root.path)
        .collect::<BTreeSet<_>>();
    let temporary_package_roots = TEMPORARY_PACKAGE_ROOT_ALLOWLIST
        .iter()
        .map(|root| (root.package, root.path))
        .collect::<BTreeSet<_>>();

    package_roots
        .iter()
        .filter(|(package, path)| {
            !target_paths.contains(path.as_str())
                && !temporary_package_roots.contains(&(package.as_str(), path.as_str()))
        })
        .map(|(package, path)| ForbiddenPackageRoot {
            package: package.clone(),
            path: path.clone(),
            rule: PACKAGE_ROOT_POLICY,
        })
        .collect()
}

fn print_target_package_roots() {
    println!(
        "{PACKAGE_ROOT_POLICY} target roots ({} entries):",
        TARGET_SHORT_PACKAGE_ROOTS.len()
    );
    for root in TARGET_SHORT_PACKAGE_ROOTS {
        println!("- {} => {}", root.path, root.owner);
    }
}

fn print_temporary_package_root_allowlist() {
    println!(
        "{PACKAGE_ROOT_POLICY} temporary allowlist ({} entries):",
        TEMPORARY_PACKAGE_ROOT_ALLOWLIST.len()
    );
    for root in TEMPORARY_PACKAGE_ROOT_ALLOWLIST {
        println!(
            "- {} at {} => {}",
            root.package, root.path, root.target_path
        );
    }
}

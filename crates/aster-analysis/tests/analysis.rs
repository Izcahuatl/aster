use std::path::{Path, PathBuf};

use aster_analysis::{AnalysisOptions, DiagnosticKind, analyze};

fn analyze_fixture(name: &str) -> aster_analysis::AnalysisResult {
    analyze(&AnalysisOptions::new(
        PathBuf::from("tests/fixtures").join(name),
    ))
}

#[test]
fn clean_fixture_has_expected_graph_and_no_diagnostics() {
    let result = analyze_fixture("clean");
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let graph = &result.graph;
    assert_eq!(graph.modules().len(), 8);
    assert_eq!(
        graph.dependencies(Path::new("main.lua")),
        vec![Path::new("network.lua"), Path::new("player.lua")]
    );
    assert_eq!(
        graph.dependencies(Path::new("player.lua")),
        vec![Path::new("inventory.lua")]
    );
    assert_eq!(
        graph.dependencies(Path::new("inventory.lua")),
        vec![Path::new("lib/util.lua")]
    );
    assert_eq!(
        graph.dependencies(Path::new("network.lua")),
        vec![Path::new("save.lua")]
    );
    assert_eq!(
        graph.dependencies(Path::new("save.lua")),
        vec![Path::new("config.lua")]
    );
    assert!(graph.dependencies(Path::new("config.lua")).is_empty());
    // main.lua and pkg/init.lua both have no incoming edges.
    assert_eq!(
        graph.entry_points(),
        vec![Path::new("main.lua"), Path::new("pkg/init.lua")]
    );
}

#[test]
fn cycle_fixture_reports_circular_dependency() {
    let result = analyze_fixture("cycle");
    let cycles: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::CircularDependency)
        .collect();
    assert_eq!(cycles.len(), 1, "diagnostics: {:?}", result.diagnostics);
    assert_eq!(
        cycles[0].message,
        "circular dependency: a.lua -> b.lua -> a.lua"
    );
    assert_eq!(result.diagnostics.len(), 1);
}

#[test]
fn dynamic_fixture_flags_dynamic_require() {
    let result = analyze_fixture("dynamic");
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.kind, DiagnosticKind::DynamicRequire);
    assert_eq!(diagnostic.file, Some(PathBuf::from("main.lua")));
    assert_eq!(diagnostic.line, Some(2));
    assert_eq!(diagnostic.column, Some(1));
}

#[test]
fn unresolved_fixture_flags_unresolved_require() {
    let result = analyze_fixture("unresolved");
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.kind, DiagnosticKind::UnresolvedRequire);
    assert_eq!(diagnostic.file, Some(PathBuf::from("main.lua")));
    assert!(diagnostic.message.contains("ghost"));
}

#[test]
fn unused_fixture_flags_orphan_but_not_entry_points() {
    let result = analyze_fixture("unused");
    let unused: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::UnusedModule)
        .collect();
    assert_eq!(unused.len(), 1, "diagnostics: {:?}", result.diagnostics);
    assert_eq!(unused[0].file, Some(PathBuf::from("orphan.lua")));
}

#[test]
fn bad_fixture_reports_parse_error_but_keeps_module() {
    let result = analyze_fixture("bad");
    assert!(result.diagnostics.iter().any(|d| {
        d.kind == DiagnosticKind::ParseError && d.file == Some(PathBuf::from("main.lua"))
    }));
    assert!(result.graph.modules().contains(&Path::new("main.lua")));
}

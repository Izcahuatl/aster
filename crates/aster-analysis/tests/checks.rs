use std::path::PathBuf;

use aster_analysis::{AnalysisOptions, Diagnostic, DiagnosticKind, LookupResult, check, explain};

fn check_fixture(name: &str) -> Vec<Diagnostic> {
    check(&AnalysisOptions::new(
        PathBuf::from("tests/fixtures/checks").join(name),
    ))
}

#[test]
fn sequence_flagged_fixture_produces_expected_diagnostics() {
    let diagnostics = check_fixture("sequence");
    let kinds_and_lines: Vec<(DiagnosticKind, Option<usize>)> =
        diagnostics.iter().map(|d| (d.kind, d.line)).collect();
    assert_eq!(
        kinds_and_lines,
        vec![
            (DiagnosticKind::ZeroIndexAccess, Some(2)),
            (DiagnosticKind::SparseArray, Some(6)),
            (DiagnosticKind::AmbiguousLength, Some(7)),
            (DiagnosticKind::OffByOneLoop, Some(10)),
            (DiagnosticKind::OffByOneLoop, Some(11)),
            (DiagnosticKind::ZeroIndexAccess, Some(15)),
        ],
        "diagnostics: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.file == Some(PathBuf::from("flagged.lua")))
    );
    assert_eq!(
        diagnostics[0].message,
        "zero-based index `t[0]` on sequence-like table `t`"
    );
    assert_eq!(
        diagnostics[3].message,
        "loop `for i = 0, #v` starts at 0; Lua sequences start at 1"
    );
    assert_eq!(
        diagnostics[4].message,
        "loop `for j = 1, #v - 1` skips the last element"
    );
    assert_eq!(
        diagnostics[5].message,
        "zero-based index `w[0]` on sequence-like table `w`"
    );
}

#[test]
fn returns_flagged_fixture_reports_discards_and_nil_padding() {
    let diagnostics = check_fixture("returns");
    assert_eq!(diagnostics.len(), 5, "diagnostics: {diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|d| d.kind == DiagnosticKind::MultiReturnInfo)
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.file == Some(PathBuf::from("flagged.lua")))
    );

    assert_eq!(diagnostics[0].line, Some(5));
    assert_eq!(
        diagnostics[0].message,
        "`three()` returns 3 values; 1 bound, 2 discarded"
    );
    assert_eq!(diagnostics[1].line, Some(6));
    assert_eq!(
        diagnostics[1].message,
        "`three()` returns 3 values; 2 bound, 1 discarded"
    );
    assert_eq!(diagnostics[2].line, Some(7));
    assert_eq!(
        diagnostics[2].message,
        "`one()` returns 1 value; 2 bound, 1 variable will be nil"
    );
    assert_eq!(diagnostics[3].line, Some(8));
    assert_eq!(
        diagnostics[3].message,
        "`none()` returns 0 values; 1 bound, 1 variable will be nil"
    );
    assert_eq!(diagnostics[4].line, Some(15));
    assert_eq!(
        diagnostics[4].message,
        "`outer()` returns 1 value; 2 bound, 1 variable will be nil"
    );
}

#[test]
fn cross_flagged_fixture_reports_unknown_members() {
    let diagnostics = check_fixture("cross/flagged");
    let kinds_and_lines: Vec<(DiagnosticKind, Option<usize>)> =
        diagnostics.iter().map(|d| (d.kind, d.line)).collect();
    assert_eq!(
        kinds_and_lines,
        vec![
            (DiagnosticKind::UnknownMember, Some(3)),
            (DiagnosticKind::UnknownMember, Some(4)),
            (DiagnosticKind::MultiReturnInfo, Some(8)),
            (DiagnosticKind::MultiReturnInfo, Some(11)),
        ],
        "diagnostics: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.file == Some(PathBuf::from("main.lua")))
    );
    assert_eq!(
        diagnostics[0].message,
        "unknown member `nope` on module binding `player`"
    );
    assert_eq!(
        diagnostics[1].message,
        "unknown member `missing` on module binding `player`"
    );
    assert_eq!(
        diagnostics[2].message,
        "`player.move()` returns 2 values; 1 bound, 1 discarded"
    );
    assert_eq!(
        diagnostics[3].message,
        "`player:move()` returns 2 values; 1 bound, 1 discarded"
    );
}

#[test]
fn cross_clean_fixture_stays_silent() {
    let diagnostics = check_fixture("cross/clean");
    assert_eq!(diagnostics, vec![], "diagnostics: {diagnostics:?}");
}

#[test]
fn class_fixture_resolves_instance_and_inherited_members() {
    let diagnostics = check_fixture("class");
    assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
    assert_eq!(diagnostics[0].kind, DiagnosticKind::UnknownMember);
    assert_eq!(diagnostics[0].line, Some(8));
    assert_eq!(
        diagnostics[0].message,
        "unknown member `missing` on instance `player`"
    );
}

#[test]
fn performance_flagged_fixture_reports_inefficiencies() {
    let diagnostics = check_fixture("performance/flagged");
    let kinds_and_lines: Vec<(DiagnosticKind, Option<usize>)> =
        diagnostics.iter().map(|d| (d.kind, d.line)).collect();
    assert_eq!(
        kinds_and_lines,
        vec![
            (DiagnosticKind::GlobalInLoop, Some(3)),
            (DiagnosticKind::TableAllocationInLoop, Some(4)),
            (DiagnosticKind::StringConcatInLoop, Some(5)),
        ],
        "diagnostics: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.file == Some(PathBuf::from("main.lua")))
    );
    assert!(
        diagnostics[0]
            .message
            .contains("repeated global lookup `math.sqrt`")
    );
    assert!(diagnostics[1].message.contains("table constructor `{}`"));
    assert!(diagnostics[2].message.contains("string concatenation `..`"));
}

#[test]
fn explain_reports_the_inheritance_walk() {
    let root = PathBuf::from("tests/fixtures/checks/class");
    let explanations = explain(&AnalysisOptions::new(&root), "main.lua", 6);
    assert_eq!(explanations.len(), 1, "explanations: {explanations:?}");
    assert_eq!(explanations[0].expression, "player.describe");
    assert_eq!(explanations[0].start_column, 7);
    assert_eq!(explanations[0].end_column, 22);
    assert!(matches!(explanations[0].result, LookupResult::Found(_)));
    assert!(
        explanations[0]
            .steps
            .iter()
            .any(|step| step == "constructor parent: Entity.new()")
    );
    assert!(
        explanations[0]
            .steps
            .iter()
            .any(|step| step == "found direct member on entity.lua")
    );
}

#[test]
fn explain_keeps_unknown_member_accesses_inspectable() {
    let root = PathBuf::from("tests/fixtures/checks/class");
    let explanations = explain(&AnalysisOptions::new(&root), "main.lua", 9);
    assert_eq!(explanations.len(), 1, "explanations: {explanations:?}");
    assert_eq!(explanations[0].expression, "mystery.member");
    assert!(matches!(explanations[0].result, LookupResult::Unknown(_)));
    assert!(
        explanations[0]
            .steps
            .iter()
            .any(|step| step.contains("no known class binding"))
    );
}

#[test]
fn explain_reports_members_inside_the_exported_class_file() {
    let root = PathBuf::from("tests/fixtures/checks/class");

    let class_member = explain(&AnalysisOptions::new(&root), "player.lua", 4);
    assert_eq!(class_member.len(), 1, "explanations: {class_member:?}");
    assert_eq!(class_member[0].expression, "Player.__index");
    assert!(!class_member[0].steps.is_empty());

    let self_member = explain(&AnalysisOptions::new(&root), "player.lua", 14);
    assert_eq!(self_member.len(), 1, "explanations: {self_member:?}");
    assert_eq!(self_member[0].expression, "self.score");
    assert!(matches!(self_member[0].result, LookupResult::Found(_)));
}

#[test]
fn performance_clean_fixture_stays_silent() {
    let diagnostics = check_fixture("performance/clean");
    assert_eq!(diagnostics, vec![], "diagnostics: {diagnostics:?}");
}

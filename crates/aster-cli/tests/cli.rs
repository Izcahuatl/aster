use assert_cmd::Command;

#[test]
fn graph_json_reports_clean_fixture() {
    let assert = Command::cargo_bin("aster")
        .unwrap()
        .args(["graph", "../aster-analysis/tests/fixtures/clean", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["diagnostics"], serde_json::json!([]));

    let modules = json["modules"].as_array().unwrap();
    let main = modules
        .iter()
        .find(|m| m["path"] == "main.lua")
        .expect("main.lua module missing from JSON output");
    assert_eq!(
        main["dependencies"],
        serde_json::json!(["network.lua", "player.lua"])
    );
}

#[test]
fn graph_fails_on_missing_directory() {
    Command::cargo_bin("aster")
        .unwrap()
        .args(["graph", "does-not-exist"])
        .assert()
        .failure();
}

#[test]
fn check_json_reports_sequence_fixture() {
    let assert = Command::cargo_bin("aster")
        .unwrap()
        .args([
            "check",
            "../aster-analysis/tests/fixtures/checks/sequence",
            "--json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let kinds: Vec<&str> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "zero_index_access",
            "sparse_array",
            "ambiguous_length",
            "off_by_one_loop",
            "off_by_one_loop",
            "zero_index_access",
        ]
    );
}

#[test]
fn check_json_reports_returns_fixture() {
    let assert = Command::cargo_bin("aster")
        .unwrap()
        .args([
            "check",
            "../aster-analysis/tests/fixtures/checks/returns",
            "--json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 5);
    assert!(diagnostics.iter().all(|d| d["kind"] == "multi_return_info"));
}

#[test]
fn check_text_reports_no_issues_when_clean() {
    // The module-graph `clean` fixture has no check findings.
    Command::cargo_bin("aster")
        .unwrap()
        .args(["check", "../aster-analysis/tests/fixtures/clean"])
        .assert()
        .success()
        .stdout("No issues found.\n");
}

#[test]
fn check_fails_on_missing_directory() {
    Command::cargo_bin("aster")
        .unwrap()
        .args(["check", "does-not-exist"])
        .assert()
        .failure();
}

#[test]
fn explain_prints_inherited_lookup_trace() {
    Command::cargo_bin("aster")
        .unwrap()
        .args([
            "explain",
            "../aster-analysis/tests/fixtures/checks/class",
            "main.lua",
            "6",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("player.describe"))
        .stdout(predicates::str::contains(
            "constructor parent: Entity.new()",
        ))
        .stdout(predicates::str::contains("resolved in entity.lua"));
}

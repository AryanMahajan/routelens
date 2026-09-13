//! The sample flow shipped for the FastAPI fixture stays runnable: it parses, it validates,
//! and every step built from a discovered endpoint names one the scan actually finds.

use rl_core::RouteLens;
use rl_model::{Flow, NodeKind};
use rl_workspace::WorkspaceKind;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/fastapi")
}

#[test]
fn the_sample_flow_matches_the_fixture_api() {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/examples/fastapi-user-lifecycle.yaml"),
    )
    .unwrap();
    let flow: Flow = yaml_serde::from_str(&text).unwrap();
    flow.validate().unwrap();

    // Scan from a throwaway copy of the workspace so the fixture's own `.routelens/` — the
    // one a developer plays with — is never touched by a test.
    let home = tempfile::TempDir::new().unwrap();
    let mut app = RouteLens::with_data_dir(home.path().join("data"));
    let root = tempfile::TempDir::new().unwrap();
    copy_dir(&fixture().join("app"), &root.path().join("app"));
    for file in ["Procfile", "requirements.txt"] {
        std::fs::copy(fixture().join(file), root.path().join(file)).unwrap();
    }
    app.create_workspace(root.path(), "fixture", WorkspaceKind::Project)
        .unwrap();
    let scan = app.scan().unwrap();
    let ids: Vec<&str> = scan.endpoints.iter().map(|e| e.id.as_str()).collect();

    for node in &flow.nodes {
        if let NodeKind::Request { request, .. } = &node.kind {
            if let Some(spec) = &request.spec_ref {
                assert!(
                    ids.contains(&spec.as_str()),
                    "{} names {spec}, which the scan did not find; it found {ids:#?}",
                    node.name.as_deref().unwrap_or("a step")
                );
            }
        }
    }
    assert!(
        flow.nodes.len() >= 6,
        "the example is meant to be a real chain"
    );

    // With the fixture app running — `uvicorn app.main:app --port 9000` in the fixture
    // directory — and ROUTELENS_FIXTURE_URL=http://localhost:9000, run the flow for real.
    let Ok(base_url) = std::env::var("ROUTELENS_FIXTURE_URL") else {
        return;
    };
    let mut env = rl_workspace::Environment::new("local");
    env.set("base_url", base_url);
    app.save_environment(&env).unwrap();
    app.set_active_environment(Some("local")).unwrap();
    app.set_secret("api_token", "letmein").unwrap();

    let run = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            app.prepare_flow(flow, rl_flow::RunOptions::default())
                .unwrap()
                .run(&mut |_| {}),
        )
        .unwrap();
    assert!(run.passed(), "{:#?}", run.results);
    // Everything ran except the arm the condition did not take.
    assert_eq!(run.summary.skipped, 1, "{:#?}", run.summary);
    assert!(run.variables.contains_key("user_id"));
    // The inputs block ran first and the summary block read everything back.
    assert_eq!(run.results[0].node.as_str(), "inputs");
    let summary = run
        .results
        .iter()
        .find(|r| r.node.as_str() == "summary")
        .unwrap();
    assert_eq!(
        summary.output.as_deref(),
        Some(
            format!(
                "Created Dana as user {} (Dana@example.com), then deleted them again.",
                run.variables["user_id"]
            )
            .as_str()
        )
    );
    assert_eq!(
        run.variables["first_item"], "item-4",
        "page 2 of 3-per-page starts at item 4"
    );
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            if entry.file_name() != "__pycache__" {
                copy_dir(&path, &target);
            }
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

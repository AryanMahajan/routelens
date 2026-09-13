//! Running a flow.
//!
//! Nodes execute one at a time in [`Flow::execution_order`]. Before each one the runner
//! looks at the edges into it and decides whether it runs at all:
//!
//! - an edge from a node that **failed** — or was skipped because something upstream of it
//!   failed — is *dead*, and one dead edge is enough to skip the node. A test step whose
//!   prerequisite did not happen must not run against half-set-up state.
//! - an edge out of a condition's untaken output, or from a node that was itself skipped by
//!   a branch, is *inactive*. A node runs as long as **one** of its edges is live, so the
//!   two arms of a condition can rejoin.
//! - a node with no edges into it always runs.
//!
//! A request node passes when it was sent, every extraction found its value, and every
//! assertion holds. Its extracted values become `{{variables}}` for everything after it —
//! they take precedence over environment variables of the same name, and are committed only
//! when the node passed.
//!
//! The runner never touches the network itself: it sends through a [`Sender`], which is
//! [`rl_http::HttpEngine`] in the application and a scripted fake in tests.

use crate::extract;
use rl_http::{Exchange, HttpEngine};
use rl_model::{
    Assertion, Flow, FlowError, Node, NodeId, NodeKind, Operator, RequestDraft, ValueSource,
    VariableContext, HANDLE_FALSE, HANDLE_TRUE,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Something that can send a resolved request.
pub trait Sender: Sync {
    fn send(&self, draft: &RequestDraft) -> impl Future<Output = rl_http::Result<Exchange>> + Send;
}

impl Sender for HttpEngine {
    fn send(&self, draft: &RequestDraft) -> impl Future<Output = rl_http::Result<Exchange>> + Send {
        self.execute(draft)
    }
}

/// Why a node did not pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Failure {
    /// Reported before anything is sent, all together, so they are fixed in one pass.
    UndefinedVariables {
        names: Vec<String>,
    },
    /// A `{{variable}}` chain could not be resolved — a cycle, or nesting too deep.
    Resolve {
        message: String,
    },
    /// The request never produced a response.
    Transport {
        message: String,
    },
    /// An extraction found nothing, so later nodes would run with a missing variable.
    Extraction {
        name: String,
        message: String,
    },
    Assertions {
        failed: usize,
    },
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::UndefinedVariables { names } => {
                format!("undefined variable(s): {}", names.join(", "))
            }
            Failure::Resolve { message } | Failure::Transport { message } => message.clone(),
            Failure::Extraction { name, message } => {
                format!("could not extract `{name}`: {message}")
            }
            Failure::Assertions { failed } => {
                format!(
                    "{failed} assertion{} failed",
                    if *failed == 1 { "" } else { "s" }
                )
            }
        }
    }
}

/// Why a node was not run at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkipReason {
    /// `node` is the one that actually failed, however far upstream.
    UpstreamFailed { node: NodeId },
    /// Every edge in came out of a condition output that was not taken.
    BranchNotTaken { node: NodeId, handle: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed { failure: Failure },
    Skipped { reason: SkipReason },
}

impl Outcome {
    pub fn is_passed(&self) -> bool {
        matches!(self, Outcome::Passed)
    }
}

/// One assertion, evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub assertion: Assertion,
    /// The right-hand side with variables resolved — what was actually compared against.
    pub expected: String,
    /// What the source yielded; absent when there was nothing there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    pub passed: bool,
    /// Why it could not be evaluated, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extracted {
    pub name: String,
    pub value: String,
}

/// Everything one node produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeResult {
    pub node: NodeId,
    #[serde(flatten)]
    pub outcome: Outcome,
    pub duration_ms: u64,
    /// The request as it was resolved and attempted. Absent for a skipped node, a condition,
    /// or a node that failed before resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<Exchange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extracted: Vec<Extracted>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<AssertionResult>,
    /// Which output a condition took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// A condition's two sides as compared, for the card to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compared: Option<(String, String)>,
}

impl NodeResult {
    fn skipped(node: &NodeId, reason: SkipReason) -> Self {
        NodeResult {
            node: node.clone(),
            outcome: Outcome::Skipped { reason },
            duration_ms: 0,
            request: None,
            exchange: None,
            extracted: Vec::new(),
            assertions: Vec::new(),
            branch: None,
            compared: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// A finished run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowRun {
    pub flow: String,
    /// Unix milliseconds.
    pub started_at: u64,
    pub duration_ms: u64,
    /// In execution order.
    pub results: Vec<NodeResult>,
    /// Every variable the run extracted, as it stood at the end.
    pub variables: BTreeMap<String, String>,
    pub summary: Summary,
}

impl FlowRun {
    pub fn passed(&self) -> bool {
        self.summary.failed == 0
    }

    pub fn result(&self, node: &NodeId) -> Option<&NodeResult> {
        self.results.iter().find(|r| &r.node == node)
    }
}

/// What a run reports as it goes, so a UI can light nodes up one by one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum FlowEvent {
    Started { order: Vec<NodeId> },
    NodeStarted { node: NodeId },
    NodeFinished { result: Box<NodeResult> },
    Finished { run: Box<FlowRun> },
}

/// Run `flow` against `base` variables, sending through `sender`.
///
/// Fails only for a flow that cannot be run at all — a cycle, a dangling edge. Everything
/// that goes wrong *while* running is a node outcome, not an error.
pub async fn run<S: Sender>(
    flow: &Flow,
    base: &VariableContext,
    sender: &S,
    on_event: &mut (dyn FnMut(FlowEvent) + Send),
) -> Result<FlowRun, FlowError> {
    flow.validate()?;
    let order = flow.execution_order()?;
    on_event(FlowEvent::Started {
        order: order.clone(),
    });

    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let clock = Instant::now();

    let mut variables: BTreeMap<String, String> = BTreeMap::new();
    let mut results: Vec<NodeResult> = Vec::with_capacity(order.len());
    let mut summary = Summary::default();

    for id in &order {
        let node = flow
            .node(id)
            .expect("execution order names only known nodes");

        let result = match gate(flow, node, &results) {
            Gate::Skip(reason) => NodeResult::skipped(id, reason),
            Gate::Run => {
                on_event(FlowEvent::NodeStarted { node: id.clone() });
                let mut ctx = base.clone();
                ctx.request_local = variables.clone();
                let result = execute(node, &ctx, sender).await;
                if result.outcome.is_passed() {
                    for e in &result.extracted {
                        variables.insert(e.name.clone(), e.value.clone());
                    }
                }
                result
            }
        };

        match &result.outcome {
            Outcome::Passed => summary.passed += 1,
            Outcome::Failed { .. } => summary.failed += 1,
            Outcome::Skipped { .. } => summary.skipped += 1,
        }
        on_event(FlowEvent::NodeFinished {
            result: Box::new(result.clone()),
        });
        results.push(result);
    }

    let run = FlowRun {
        flow: flow.name.clone(),
        started_at,
        duration_ms: clock.elapsed().as_millis() as u64,
        results,
        variables,
        summary,
    };
    on_event(FlowEvent::Finished {
        run: Box::new(run.clone()),
    });
    Ok(run)
}

enum Gate {
    Run,
    Skip(SkipReason),
}

/// Look at every edge into `node` and decide whether it runs. See the module docs.
fn gate(flow: &Flow, node: &Node, finished: &[NodeResult]) -> Gate {
    let mut any_live = false;
    let mut first_inactive: Option<SkipReason> = None;
    let mut edges = 0;

    for edge in flow.upstream(&node.id) {
        edges += 1;
        let Some(upstream) = finished.iter().find(|r| r.node == edge.from) else {
            // Cannot happen after validation: the order guarantees `from` finished first.
            continue;
        };

        match &upstream.outcome {
            Outcome::Failed { .. } => {
                return Gate::Skip(SkipReason::UpstreamFailed {
                    node: edge.from.clone(),
                });
            }
            Outcome::Skipped {
                reason: SkipReason::UpstreamFailed { node },
            } => {
                return Gate::Skip(SkipReason::UpstreamFailed { node: node.clone() });
            }
            Outcome::Skipped { reason } => {
                first_inactive.get_or_insert_with(|| reason.clone());
            }
            Outcome::Passed => match (&upstream.branch, &edge.handle) {
                // A condition: only the edge out of the taken output is live.
                (Some(taken), Some(handle)) if taken != handle => {
                    first_inactive.get_or_insert_with(|| SkipReason::BranchNotTaken {
                        node: edge.from.clone(),
                        handle: handle.clone(),
                    });
                }
                _ => any_live = true,
            },
        }
    }

    if edges == 0 || any_live {
        Gate::Run
    } else {
        Gate::Skip(first_inactive.unwrap_or(SkipReason::BranchNotTaken {
            node: node.id.clone(),
            handle: String::new(),
        }))
    }
}

async fn execute<S: Sender>(node: &Node, ctx: &VariableContext, sender: &S) -> NodeResult {
    let clock = Instant::now();
    let mut result = NodeResult {
        node: node.id.clone(),
        outcome: Outcome::Passed,
        duration_ms: 0,
        request: None,
        exchange: None,
        extracted: Vec::new(),
        assertions: Vec::new(),
        branch: None,
        compared: None,
    };

    match &node.kind {
        NodeKind::Request {
            request,
            extract,
            assert,
        } => {
            result.outcome = run_request(request, extract, assert, ctx, sender, &mut result).await;
        }
        NodeKind::Condition { left, op, right } => {
            result.outcome = run_condition(left, *op, right, ctx, &mut result);
        }
    }

    result.duration_ms = clock.elapsed().as_millis() as u64;
    result
}

async fn run_request<S: Sender>(
    request: &RequestDraft,
    extract: &[rl_model::Extraction],
    assert: &[Assertion],
    ctx: &VariableContext,
    sender: &S,
    result: &mut NodeResult,
) -> Outcome {
    let undefined: Vec<String> = request
        .variable_references()
        .into_iter()
        .filter(|name| !ctx.is_defined(name))
        .collect();
    if !undefined.is_empty() {
        return Outcome::Failed {
            failure: Failure::UndefinedVariables { names: undefined },
        };
    }

    let resolved = match request.resolve(ctx) {
        Ok((resolved, _secrets)) => resolved,
        Err(error) => {
            return Outcome::Failed {
                failure: Failure::Resolve {
                    message: error.to_string(),
                },
            }
        }
    };
    result.request = Some(resolved.clone());

    let exchange = match sender.send(&resolved).await {
        Ok(exchange) => exchange,
        Err(error) => {
            return Outcome::Failed {
                failure: Failure::Transport {
                    message: flatten(&error),
                },
            }
        }
    };

    // Extract first, then assert — so a failed assertion still shows what was extracted,
    // and a missing extraction still shows which assertions would have held.
    let mut first_failure: Option<Failure> = None;
    for extraction in extract {
        match extract::value(&extraction.source, &exchange) {
            Ok(Some(value)) => result.extracted.push(Extracted {
                name: extraction.name.clone(),
                value,
            }),
            Ok(None) => {
                first_failure.get_or_insert(Failure::Extraction {
                    name: extraction.name.clone(),
                    message: format!("nothing at {}", describe(&extraction.source)),
                });
            }
            Err(error) => {
                first_failure.get_or_insert(Failure::Extraction {
                    name: extraction.name.clone(),
                    message: error.to_string(),
                });
            }
        }
    }

    let mut failed = 0;
    for assertion in assert {
        let evaluated = evaluate(assertion, &exchange, ctx);
        if !evaluated.passed {
            failed += 1;
        }
        result.assertions.push(evaluated);
    }
    result.exchange = Some(exchange);

    if let Some(failure) = first_failure {
        return Outcome::Failed { failure };
    }
    if failed > 0 {
        return Outcome::Failed {
            failure: Failure::Assertions { failed },
        };
    }
    Outcome::Passed
}

fn evaluate(assertion: &Assertion, exchange: &Exchange, ctx: &VariableContext) -> AssertionResult {
    let mut out = AssertionResult {
        assertion: assertion.clone(),
        expected: assertion.expected.clone(),
        actual: None,
        passed: false,
        error: None,
    };

    let expected = if assertion.op.is_unary() {
        String::new()
    } else {
        match ctx.resolve(&assertion.expected) {
            Ok(resolved) => resolved.value,
            Err(error) => {
                out.error = Some(error.to_string());
                return out;
            }
        }
    };
    out.expected = expected;

    match extract::value(&assertion.source, exchange) {
        Ok(actual) => {
            out.passed = crate::compare::holds(assertion.op, actual.as_deref(), &out.expected);
            out.actual = actual;
        }
        Err(error) => {
            out.error = Some(error.to_string());
        }
    }
    out
}

fn run_condition(
    left: &str,
    op: Operator,
    right: &str,
    ctx: &VariableContext,
    result: &mut NodeResult,
) -> Outcome {
    let undefined: Vec<String> = VariableContext::references(left)
        .into_iter()
        .chain(VariableContext::references(right))
        .filter(|name| !ctx.is_defined(name))
        .collect();
    if !undefined.is_empty() {
        return Outcome::Failed {
            failure: Failure::UndefinedVariables { names: undefined },
        };
    }

    let resolve = |text: &str| ctx.resolve(text).map(|r| r.value);
    let (left, right) = match (resolve(left), resolve(right)) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => {
            return Outcome::Failed {
                failure: Failure::Resolve {
                    message: e.to_string(),
                },
            }
        }
    };

    // An empty left side is "nothing there", which is what `exists` needs to be able to see.
    let actual = if left.is_empty() {
        None
    } else {
        Some(left.as_str())
    };
    let taken = if crate::compare::holds(op, actual, &right) {
        HANDLE_TRUE
    } else {
        HANDLE_FALSE
    };
    result.branch = Some(taken.to_string());
    result.compared = Some((left, right));
    Outcome::Passed
}

/// A value source, as a person would name it in a message.
pub fn describe(source: &ValueSource) -> String {
    match source {
        ValueSource::Status => "status".to_string(),
        ValueSource::Duration => "duration".to_string(),
        ValueSource::BodyText => "body".to_string(),
        ValueSource::Header { header } => format!("header {header}"),
        ValueSource::Body { path } => format!("body path `{path}`"),
    }
}

/// The whole chain, because "request failed" alone is not actionable.
fn flatten(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(current) = source {
        let text = current.to_string();
        if !parts.contains(&text) {
            parts.push(text);
        }
        source = current.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_http::{Body, HttpError, Response, SentRequest, Timing};
    use rl_model::{BodyValue, Extraction, HttpMethod, KeyValue, Operator};
    use std::sync::Mutex;

    type Handler = Box<dyn Fn(&RequestDraft) -> rl_http::Result<Exchange> + Send + Sync>;

    /// A server that never listens: a closure decides what each request gets back.
    struct Fake {
        handler: Handler,
        seen: Mutex<Vec<RequestDraft>>,
    }

    impl Fake {
        fn new(
            handler: impl Fn(&RequestDraft) -> rl_http::Result<Exchange> + Send + Sync + 'static,
        ) -> Self {
            Fake {
                handler: Box::new(handler),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url_with_path_values())
                .collect()
        }
    }

    impl Sender for Fake {
        async fn send(&self, draft: &RequestDraft) -> rl_http::Result<Exchange> {
            self.seen.lock().unwrap().push(draft.clone());
            (self.handler)(draft)
        }
    }

    fn reply(status: u16, body: &str) -> rl_http::Result<Exchange> {
        Ok(Exchange {
            request: SentRequest {
                method: "GET".into(),
                url: String::new(),
                headers: vec![],
                body_size: 0,
                body_preview: None,
            },
            response: Response {
                status,
                status_text: String::new(),
                headers: vec![("content-type".into(), "application/json".into())],
                body: Body {
                    bytes: body.as_bytes().to_vec(),
                    truncated: false,
                    reported_length: None,
                    content_type: Some("application/json".into()),
                    content_encoding: None,
                },
                timing: Timing::default(),
                redirects: vec![],
                insecure: false,
            },
        })
    }

    fn get(url: &str) -> Node {
        let mut node = Node::request(RequestDraft::new(HttpMethod::Get, url));
        if let NodeKind::Request { assert, .. } = &mut node.kind {
            assert.push(Assertion::status_ok());
        }
        node
    }

    fn extracting(mut node: Node, name: &str, path: &str) -> Node {
        if let NodeKind::Request { extract, .. } = &mut node.kind {
            extract.push(Extraction {
                name: name.into(),
                source: ValueSource::Body { path: path.into() },
            });
        }
        node
    }

    fn asserting(mut node: Node, source: ValueSource, op: Operator, expected: &str) -> Node {
        if let NodeKind::Request { assert, .. } = &mut node.kind {
            assert.push(Assertion {
                source,
                op,
                expected: expected.into(),
            });
        }
        node
    }

    fn env() -> VariableContext {
        let mut ctx = VariableContext::new();
        ctx.environment
            .insert("base_url".into(), "http://api.test".into());
        ctx
    }

    async fn run_quietly<S: Sender>(flow: &Flow, ctx: &VariableContext, sender: &S) -> FlowRun {
        let mut events = Vec::new();
        run(flow, ctx, sender, &mut |e| events.push(e))
            .await
            .unwrap()
    }

    fn outcome_of<'a>(run: &'a FlowRun, id: &NodeId) -> &'a Outcome {
        &run.result(id).unwrap().outcome
    }

    /// The brief's first example: login → me → user, each step feeding the next.
    #[tokio::test]
    async fn extracted_values_flow_into_later_requests() {
        let api = Fake::new(|r| match r.url_with_path_values().as_str() {
            "http://api.test/auth/login" => reply(200, r#"{"access_token":"tok-1"}"#),
            "http://api.test/me" => {
                assert_eq!(
                    r.headers
                        .iter()
                        .find(|h| h.key == "Authorization")
                        .map(|h| h.value.as_str()),
                    Some("Bearer tok-1")
                );
                reply(200, r#"{"user":{"id":42}}"#)
            }
            "http://api.test/users/42" => reply(200, r#"{"id":42,"name":"Aryan"}"#),
            other => panic!("unexpected request to {other}"),
        });

        let mut flow = Flow::new("auth");
        let mut login = RequestDraft::new(HttpMethod::Post, "{{base_url}}/auth/login");
        login.body = BodyValue::Json {
            content: r#"{"email":"a@b.c"}"#.into(),
        };
        let login = flow.add(extracting(
            Node::request(login),
            "auth_token",
            "access_token",
        ));

        let mut me = RequestDraft::new(HttpMethod::Get, "{{base_url}}/me");
        me.headers
            .push(KeyValue::new("Authorization", "Bearer {{auth_token}}"));
        let me = flow.add(extracting(Node::request(me), "user_id", "user.id"));

        let user = flow.add(asserting(
            get("{{base_url}}/users/{{user_id}}"),
            ValueSource::Body {
                path: "name".into(),
            },
            Operator::Equals,
            "Aryan",
        ));
        flow.connect(&login, &me);
        flow.connect(&me, &user);

        let run = run_quietly(&flow, &env(), &api).await;
        assert!(run.passed(), "{:#?}", run.results);
        assert_eq!(
            run.summary,
            Summary {
                passed: 3,
                failed: 0,
                skipped: 0
            }
        );
        assert_eq!(run.variables["auth_token"], "tok-1");
        assert_eq!(run.variables["user_id"], "42");
        assert_eq!(
            api.urls(),
            vec![
                "http://api.test/auth/login",
                "http://api.test/me",
                "http://api.test/users/42"
            ]
        );

        let last = run.result(&user).unwrap();
        assert_eq!(
            last.assertions.len(),
            2,
            "the status check and the name check"
        );
        assert_eq!(last.assertions[1].actual.as_deref(), Some("Aryan"));
        assert!(last.request.is_some() && last.exchange.is_some());
    }

    #[tokio::test]
    async fn a_failed_node_skips_everything_that_depends_on_it_and_names_the_culprit() {
        let api = Fake::new(|r| {
            if r.url.ends_with("/login") {
                reply(401, r#"{"detail":"bad credentials"}"#)
            } else {
                reply(200, "{}")
            }
        });

        let mut flow = Flow::new("chain");
        let login = flow.add(get("{{base_url}}/login"));
        let me = flow.add(get("{{base_url}}/me"));
        let user = flow.add(get("{{base_url}}/user"));
        let unrelated = flow.add(get("{{base_url}}/health"));
        flow.connect(&login, &me);
        flow.connect(&me, &user);

        let run = run_quietly(&flow, &env(), &api).await;
        assert_eq!(
            run.summary,
            Summary {
                passed: 1,
                failed: 1,
                skipped: 2
            }
        );
        assert!(matches!(
            outcome_of(&run, &login),
            Outcome::Failed {
                failure: Failure::Assertions { failed: 1 }
            }
        ));
        // Both skipped nodes point at login, not at the node immediately before them.
        for id in [&me, &user] {
            assert_eq!(
                outcome_of(&run, id),
                &Outcome::Skipped {
                    reason: SkipReason::UpstreamFailed {
                        node: login.clone()
                    }
                }
            );
        }
        assert!(outcome_of(&run, &unrelated).is_passed());
        assert_eq!(api.urls().len(), 2, "skipped nodes were never sent");
    }

    #[tokio::test]
    async fn an_undefined_variable_fails_the_node_before_anything_is_sent() {
        let api = Fake::new(|_| reply(200, "{}"));
        let mut flow = Flow::new("undefined");
        let node = flow.add(get("{{base_url}}/users/{{user_id}}"));

        let run = run_quietly(&flow, &env(), &api).await;
        assert_eq!(
            outcome_of(&run, &node),
            &Outcome::Failed {
                failure: Failure::UndefinedVariables {
                    names: vec!["user_id".into()]
                }
            }
        );
        assert!(api.urls().is_empty());
    }

    #[tokio::test]
    async fn a_transport_error_is_a_failure_with_the_whole_message() {
        let api = Fake::new(|_| Err(HttpError::Timeout { ms: 30 }));
        let mut flow = Flow::new("timeout");
        let node = flow.add(get("{{base_url}}/slow"));

        let run = run_quietly(&flow, &env(), &api).await;
        match outcome_of(&run, &node) {
            Outcome::Failed {
                failure: Failure::Transport { message },
            } => assert!(message.contains("timed out"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert!(
            run.result(&node).unwrap().request.is_some(),
            "what was attempted is kept"
        );
    }

    #[tokio::test]
    async fn an_extraction_that_finds_nothing_fails_the_node_but_assertions_still_report() {
        let api = Fake::new(|_| reply(200, r#"{"user":{}}"#));
        let mut flow = Flow::new("extract");
        let node = flow.add(extracting(get("{{base_url}}/me"), "user_id", "user.id"));

        let run = run_quietly(&flow, &env(), &api).await;
        let result = run.result(&node).unwrap();
        match &result.outcome {
            Outcome::Failed {
                failure: Failure::Extraction { name, message },
            } => {
                assert_eq!(name, "user_id");
                assert!(message.contains("user.id"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            result.assertions[0].passed,
            "the status assertion still ran"
        );
        assert!(
            run.variables.is_empty(),
            "nothing is committed from a failed node"
        );
    }

    #[tokio::test]
    async fn a_condition_sends_the_run_down_one_output_and_the_arms_can_rejoin() {
        let api = Fake::new(|r| {
            if r.url.ends_with("/me") {
                reply(200, r#"{"role":"admin"}"#)
            } else {
                reply(200, "{}")
            }
        });

        let mut flow = Flow::new("branch");
        let me = flow.add(extracting(get("{{base_url}}/me"), "role", "role"));
        let is_admin = flow.add(Node::condition("{{role}}", Operator::Equals, "admin"));
        let admin_only = flow.add(get("{{base_url}}/admin"));
        let user_only = flow.add(get("{{base_url}}/user"));
        let after = flow.add(get("{{base_url}}/logout"));
        flow.connect(&me, &is_admin);
        flow.edges
            .push(rl_model::Edge::new(&is_admin, &admin_only).via(HANDLE_TRUE));
        flow.edges
            .push(rl_model::Edge::new(&is_admin, &user_only).via(HANDLE_FALSE));
        flow.connect(&admin_only, &after);
        flow.connect(&user_only, &after);

        let run = run_quietly(&flow, &env(), &api).await;
        assert_eq!(
            run.result(&is_admin).unwrap().branch.as_deref(),
            Some("true")
        );
        assert!(outcome_of(&run, &admin_only).is_passed());
        assert_eq!(
            outcome_of(&run, &user_only),
            &Outcome::Skipped {
                reason: SkipReason::BranchNotTaken {
                    node: is_admin.clone(),
                    handle: "false".into()
                }
            }
        );
        assert!(
            outcome_of(&run, &after).is_passed(),
            "one live edge is enough for the arms to rejoin"
        );
        assert_eq!(
            run.summary,
            Summary {
                passed: 4,
                failed: 0,
                skipped: 1
            }
        );
    }

    #[tokio::test]
    async fn a_node_after_an_untaken_branch_is_skipped_by_branch_not_by_failure() {
        let api = Fake::new(|_| reply(200, r#"{"count":0}"#));
        let mut flow = Flow::new("deep-branch");
        let list = flow.add(extracting(get("{{base_url}}/items"), "count", "count"));
        let any = flow.add(Node::condition("{{count}}", Operator::GreaterThan, "0"));
        let first = flow.add(get("{{base_url}}/items/first"));
        let detail = flow.add(get("{{base_url}}/items/first/detail"));
        flow.connect(&list, &any);
        flow.edges
            .push(rl_model::Edge::new(&any, &first).via(HANDLE_TRUE));
        flow.connect(&first, &detail);

        let run = run_quietly(&flow, &env(), &api).await;
        assert!(matches!(
            outcome_of(&run, &detail),
            Outcome::Skipped {
                reason: SkipReason::BranchNotTaken { .. }
            }
        ));
        assert_eq!(run.summary.failed, 0);
    }

    #[tokio::test]
    async fn a_failure_on_one_input_beats_a_live_edge_on_another() {
        let api = Fake::new(|r| {
            if r.url.ends_with("/broken") {
                reply(500, "{}")
            } else {
                reply(200, "{}")
            }
        });
        let mut flow = Flow::new("fan-in");
        let ok = flow.add(get("{{base_url}}/ok"));
        let broken = flow.add(get("{{base_url}}/broken"));
        let join = flow.add(get("{{base_url}}/join"));
        flow.connect(&ok, &join);
        flow.connect(&broken, &join);

        let run = run_quietly(&flow, &env(), &api).await;
        assert_eq!(
            outcome_of(&run, &join),
            &Outcome::Skipped {
                reason: SkipReason::UpstreamFailed { node: broken }
            }
        );
    }

    #[tokio::test]
    async fn flow_variables_override_the_environment() {
        let api = Fake::new(|r| {
            if r.url.ends_with("/token") {
                reply(200, r#"{"token":"fresh"}"#)
            } else {
                assert!(
                    r.url_with_path_values().ends_with("/use/fresh"),
                    "{}",
                    r.url
                );
                reply(200, "{}")
            }
        });
        let mut ctx = env();
        ctx.environment.insert("token".into(), "stale".into());

        let mut flow = Flow::new("precedence");
        let a = flow.add(extracting(get("{{base_url}}/token"), "token", "token"));
        let b = flow.add(get("{{base_url}}/use/{{token}}"));
        flow.connect(&a, &b);

        let run = run_quietly(&flow, &ctx, &api).await;
        assert!(run.passed(), "{:#?}", run.results);
    }

    #[tokio::test]
    async fn assertions_resolve_variables_and_report_what_was_compared() {
        let api = Fake::new(|_| reply(200, r#"{"id":"u-9"}"#));
        let mut ctx = env();
        ctx.environment.insert("expected_id".into(), "u-9".into());

        let mut flow = Flow::new("assert-vars");
        let node = flow.add(asserting(
            get("{{base_url}}/x"),
            ValueSource::Body { path: "id".into() },
            Operator::Equals,
            "{{expected_id}}",
        ));
        let missing = flow.add(asserting(
            get("{{base_url}}/x"),
            ValueSource::Body { path: "id".into() },
            Operator::Equals,
            "{{nope}}",
        ));

        let run = run_quietly(&flow, &ctx, &api).await;
        let ok = &run.result(&node).unwrap().assertions[1];
        assert!(ok.passed);
        assert_eq!(ok.expected, "u-9");

        let bad = &run.result(&missing).unwrap().assertions[1];
        assert!(!bad.passed);
        assert!(bad.error.as_deref().unwrap().contains("nope"));
    }

    #[tokio::test]
    async fn events_arrive_in_order_and_an_invalid_flow_is_refused_up_front() {
        let api = Fake::new(|_| reply(200, "{}"));
        let mut flow = Flow::new("events");
        let a = flow.add(get("{{base_url}}/a"));
        let b = flow.add(get("{{base_url}}/b"));
        flow.connect(&a, &b);

        let mut names = Vec::new();
        run(&flow, &env(), &api, &mut |e| {
            names.push(match e {
                FlowEvent::Started { .. } => "started",
                FlowEvent::NodeStarted { .. } => "node_started",
                FlowEvent::NodeFinished { .. } => "node_finished",
                FlowEvent::Finished { .. } => "finished",
            })
        })
        .await
        .unwrap();
        assert_eq!(
            names,
            vec![
                "started",
                "node_started",
                "node_finished",
                "node_started",
                "node_finished",
                "finished"
            ]
        );

        flow.connect(&b, &a);
        let mut any = false;
        let error = run(&flow, &env(), &api, &mut |_| any = true)
            .await
            .unwrap_err();
        assert!(matches!(error, FlowError::Cycle(_)));
        assert!(!any, "nothing ran");
    }

    #[test]
    fn results_serialize_with_a_flat_status_tag_for_the_ui() {
        let result = NodeResult::skipped(
            &NodeId::from_raw("n1"),
            SkipReason::UpstreamFailed {
                node: NodeId::from_raw("n0"),
            },
        );
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "skipped");
        assert_eq!(json["reason"]["kind"], "upstream_failed");
        assert_eq!(json["reason"]["node"], "n0");

        let failed = NodeResult {
            outcome: Outcome::Failed {
                failure: Failure::Assertions { failed: 2 },
            },
            ..result
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["failure"]["kind"], "assertions");
        assert_eq!(json["failure"]["failed"], 2);
    }
}

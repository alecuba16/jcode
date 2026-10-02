use super::*;
use crate::tool::ToolExecutionMode;
use serde_json::json;

fn make_ctx(
    decision_tx: Option<tokio::sync::mpsc::UnboundedSender<DecisionInputRequest>>,
) -> ToolContext {
    ToolContext {
        session_id: "ses_test".to_string(),
        message_id: "msg_test".to_string(),
        tool_call_id: "call_test".to_string(),
        working_dir: None,
        stdin_request_tx: None,
        decision_request_tx: decision_tx,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::Direct,
    }
}

#[tokio::test]
async fn rejects_fewer_than_two_options() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}]
    });
    let output = tool.execute(input, make_ctx(None)).await.unwrap();
    assert!(
        output.output.contains("at least 2 options"),
        "expected guard message, got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn rejects_more_than_five_options() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let options: Vec<_> = (1..=6)
        .map(|i| json!({"label": format!("option {}", i)}))
        .collect();
    let input = json!({
        "intent": "pick one",
        "question": "Which one?",
        "options": options
    });
    let output = tool.execute(input, make_ctx(None)).await.unwrap();
    assert!(
        output.output.contains("at most 5 options"),
        "expected guard message, got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn without_channel_tells_agent_to_ask_in_plain_text() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });
    let output = tool.execute(input, make_ctx(None)).await.unwrap();
    assert!(
        output.output.contains("No interactive chooser"),
        "expected no-chooser message, got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn answers_option_choice_via_channel() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [
            {"label": "Postgres", "detail": "managed"},
            {"label": "SQLite"}
        ],
        "context": "schema choice blocks the migration"
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    // Server-side: receive the request, reply with option 2.
    let request = rx.recv().await.expect("decision request");
    assert_eq!(request.tool_call_id, "call_test");
    assert_eq!(request.question, "Which database?");
    assert_eq!(request.options.len(), 2);
    assert_eq!(request.options[0].label, "Postgres");
    assert_eq!(request.options[0].detail.as_deref(), Some("managed"));
    request
        .response_tx
        .send(DecisionInputResponse::Option(1))
        .unwrap();

    let output = handle.await.unwrap().unwrap();
    assert!(
        output
            .output
            .contains("User chose option 1: Postgres (value: managed)"),
        "picked option must carry its label and value, got: {}",
        output.output
    );
    assert!(
        output
            .output
            .contains("Context: schema choice blocks the migration")
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn answers_free_form_text_via_channel() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    let request = rx.recv().await.expect("decision request");
    request
        .response_tx
        .send(DecisionInputResponse::Text("use DuckDB".to_string()))
        .unwrap();

    let output = handle.await.unwrap().unwrap();
    assert!(
        output
            .output
            .contains("User answered (free form): use DuckDB"),
        "got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn dismissed_chooser_tells_agent_to_ask_in_plain_text() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    let request = rx.recv().await.expect("decision request");
    request
        .response_tx
        .send(DecisionInputResponse::Dismissed)
        .unwrap();

    let output = handle.await.unwrap().unwrap();
    assert!(
        output.output.contains("dismissed the chooser"),
        "got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn disabled_config_returns_plain_text_fallback() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "false");
    }
    crate::config::invalidate_config_cache();

    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let output = tool.execute(input, make_ctx(Some(tx))).await.unwrap();
    assert!(
        output.output.contains("disabled in config"),
        "got: {}",
        output.output
    );
    // The request must never reach the chooser channel.
    assert!(rx.try_recv().is_err());

    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn timeout_auto_picks_first_option() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISION_TIMEOUT_SECS", "1");
    }
    crate::config::invalidate_config_cache();

    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    // Server-side: receive the request but never answer, keeping the
    // response channel open so the timeout (not a closed channel) resolves.
    let _request = rx.recv().await.expect("decision request");

    let output = handle.await.unwrap().unwrap();
    assert!(
        output.output.contains("auto-picked option 1: Postgres"),
        "got: {}",
        output.output
    );
    assert!(
        output
            .output
            .contains("No answer within the configured timeout"),
        "got: {}",
        output.output
    );

    unsafe {
        std::env::remove_var("JCODE_DECISION_TIMEOUT_SECS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn zero_timeout_waits_for_user_answer() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISION_TIMEOUT_SECS", "0");
    }
    crate::config::invalidate_config_cache();

    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    // Hold the request unanswered for longer than a 1s timeout would allow,
    // then answer. With timeout 0 the tool must still be waiting.
    let request = rx.recv().await.expect("decision request");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    request
        .response_tx
        .send(DecisionInputResponse::Option(2))
        .unwrap();

    let output = handle.await.unwrap().unwrap();
    assert!(
        output.output.contains("User chose option 2: SQLite"),
        "got: {}",
        output.output
    );
    assert!(!output.output.contains("auto-picked"));

    unsafe {
        std::env::remove_var("JCODE_DECISION_TIMEOUT_SECS");
    }
    crate::config::invalidate_config_cache();
}

#[tokio::test]
async fn dropped_response_channel_ends_without_hang() {
    let _guard = crate::storage::lock_test_env();
    unsafe {
        std::env::set_var("JCODE_DECISIONS", "true");
    }
    crate::config::invalidate_config_cache();
    let tool = AskUserTool::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DecisionInputRequest>();
    let input = json!({
        "intent": "pick db",
        "question": "Which database?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    });

    let handle = {
        let ctx = make_ctx(Some(tx));
        tokio::spawn(async move { tool.execute(input, ctx).await })
    };

    let request = rx.recv().await.expect("decision request");
    drop(request.response_tx);

    let output = handle.await.unwrap().unwrap();
    assert!(
        output.output.contains("chooser was closed"),
        "got: {}",
        output.output
    );
    unsafe {
        std::env::remove_var("JCODE_DECISIONS");
    }
    crate::config::invalidate_config_cache();
}

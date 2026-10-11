// Included only by proxy::tests. scripts/check-opencode-routing.sh invokes this
// ignored smoke; the CLI receives a scrubbed environment and temporary home/state.

// The owner credential only creates/revokes the fixture token. Neither native CLI
// receives it; their actual model requests must use the issued Client-scope token.
const NATIVE_SMOKE_ADMIN_PASSWORD: &str = "native-smoke-owner";

async fn native_client_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Value,
) -> axum::response::Response {
    use axum::extract::ConnectInfo;
    use tower::ServiceExt;

    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "localhost")
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let mut req = req.body(Body::from(body.to_string())).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            50_000,
        ))));
    app.clone().oneshot(req).await.unwrap()
}

async fn native_client_token(
    app: &axum::Router,
    db: &std::path::Path,
    name: &str,
) -> (String, String) {
    let response = native_client_request(
        app,
        "POST",
        "/api/auth/tokens",
        Some(NATIVE_SMOKE_ADMIN_PASSWORD),
        json!({"name":name,"scope":"client","expires_at":crate::db::now_ms() + 600_000}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let value: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    let token = value["token"].as_str().unwrap().to_owned();
    let id = value["id"].as_str().unwrap().to_owned();
    let metadata = crate::auth::tokens::check(db, &token).await.unwrap();
    assert_eq!(metadata.scope, crate::auth::tokens::Scope::Client);
    assert_eq!(metadata.id, id);
    assert_eq!(
        native_client_request(app, "GET", "/api/accounts", Some(&token), Value::Null)
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "Client token must not authorize router administration, even on loopback"
    );
    (id, token)
}

async fn native_client_revoke(
    app: &axum::Router,
    id: &str,
    token: &str,
    model: &str,
    protocol: &str,
) {
    let response = native_client_request(
        app,
        "DELETE",
        &format!("/api/auth/tokens/{id}"),
        Some(NATIVE_SMOKE_ADMIN_PASSWORD),
        Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = if protocol == "responses" {
        json!({"model":model,"input":"A revoked token must never reach an upstream","stream":true})
    } else {
        json!({"model":model,"messages":[{"role":"user","content":"A revoked token must never reach an upstream"}],"max_tokens":8,"stream":true})
    };
    assert_eq!(
        native_client_request(app, "POST", &format!("/v1/{protocol}"), Some(token), body)
            .await
            .status(),
        StatusCode::UNAUTHORIZED,
        "revoking the exact token used by the native client must immediately reject inference"
    );
}

fn opencode_mock_stream(body: &Value, responses: bool, fixture: &std::path::Path) -> String {
    native_tool_mock_stream(body, responses, "read", json!({"filePath":fixture}))
}

fn native_tool_mock_stream(body: &Value, responses: bool, name: &str, arguments: Value) -> String {
    let model = body["model"].as_str().unwrap();
    let answered = if responses {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["type"] == "function_call_output")
    } else {
        body["messages"].as_array().unwrap().iter().any(|m| {
            m["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
        })
    };
    let args = arguments.to_string();
    let answer = format!("SMOKE_OK {model}");
    let frames = if responses {
        let item = if answered {
            json!({"type":"message","id":"msg_smoke","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":answer,"annotations":[]}]})
        } else {
            json!({"type":"function_call","id":"fc_smoke","call_id":"toolu_smoke","name":name,"arguments":args,"status":"completed"})
        };
        let response = json!({"id":"resp_smoke","object":"response","status":"completed","model":model,
            "output":[item],"usage":{"input_tokens":42,"output_tokens":13,"total_tokens":55}});
        let mut frames = vec![
            json!({"type":"response.created","response":{"id":"resp_smoke","model":model,"status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"item":if answered {
                json!({"type":"message","id":"msg_smoke","role":"assistant","status":"in_progress","content":[]})
            } else { json!({"type":"function_call","id":"fc_smoke","call_id":"toolu_smoke","name":name,"arguments":"","status":"in_progress"}) }}),
        ];
        if answered {
            frames.push(json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_smoke","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
            frames.push(json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_smoke","content_index":0,"delta":answer}));
            frames.push(json!({"type":"response.output_text.done","output_index":0,"item_id":"msg_smoke","content_index":0,"text":answer}));
            frames.push(json!({"type":"response.content_part.done","output_index":0,"item_id":"msg_smoke","content_index":0,"part":item["content"][0]}));
        } else {
            frames.push(json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_smoke","delta":args}));
            frames.push(json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_smoke","arguments":args}));
        }
        frames.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
        frames.push(json!({"type":"response.completed","response":response}));
        frames
    } else {
        vec![
            json!({"type":"message_start","message":{"id":"msg_smoke","type":"message","role":"assistant","model":model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":42,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":if answered {json!({"type":"text","text":""})} else {json!({"type":"tool_use","id":"toolu_smoke","name":name,"input":{}})}}),
            json!({"type":"content_block_delta","index":0,"delta":if answered {json!({"type":"text_delta","text":answer})} else {json!({"type":"input_json_delta","partial_json":args})}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":if answered {"end_turn"} else {"tool_use"},"stop_sequence":null},"usage":{"output_tokens":13}}),
            json!({"type":"message_stop"}),
        ]
    };
    frames
        .iter()
        .map(|frame| {
            format!(
                "event: {}\ndata: {frame}\n\n",
                frame["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires OpenCode; run scripts/check-opencode-routing.sh"]
async fn the_real_opencode_cli_routes_three_providers() {
    use axum::routing::post;
    use std::time::Duration;

    let cli = std::env::var_os("AR_OPENCODE_BIN")
        .expect("set AR_OPENCODE_BIN via scripts/check-opencode-routing.sh");
    let lab = Lab::local("opencode-routing");
    let scratch = lab.state.config.db_path.parent().unwrap().join("opencode");
    for dir in ["home", "config", "state", "cache", "data", "tmp", "project"] {
        std::fs::create_dir_all(scratch.join(dir)).unwrap();
    }
    let project = scratch.join("project").canonicalize().unwrap();
    let fixture = project.join("fixture.txt");
    std::fs::write(&fixture, "LOCAL_TOOL_FIXTURE_OK\n").unwrap();
    let (observed, mut requests) = tokio::sync::mpsc::channel(16);
    let handler = move |req: Request<Body>| {
        let observed = observed.clone();
        let fixture = fixture.clone();
        async move {
            let (parts, body) = req.into_parts();
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(body, MAX_REQUEST_BODY).await.unwrap(),
            )
            .unwrap();
            let responses = parts.uri.path() == "/v1/responses";
            let output = opencode_mock_stream(&body, responses, &fixture);
            observed
                .send((parts.uri.path().to_owned(), parts.headers, body))
                .await
                .unwrap();
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(output))
                .unwrap()
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    // An outbound proxy trap blocks OpenCode's optional npm registry checks. The
    // configured models and read tool need only the local servers in this test.
    let fake = axum::Router::new()
        .route("/v1/messages", post(handler.clone()))
        .route("/v1/responses", post(handler))
        .fallback(|| async { StatusCode::FORBIDDEN });
    let fake_server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let config = Config {
        server: true,
        bind: ([127, 0, 0, 1], 0).into(),
        upstream: upstream.clone(),
        db_path: lab.state.config.db_path.clone(),
        body_cap: 65536,
        dump_headers: 0,
        allow_hosts: vec![],
        public_url: None,
        push_keys_dir: None,
        agents_dir: None,
        drain: Duration::from_secs(1),
        stop_delay: Duration::ZERO,
    };
    let (capture, mut captured) = tokio::sync::mpsc::channel(16);
    let (network, _) = crate::network::switch();
    let mut state = AppState::new(config, capture, network).unwrap();
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    Arc::get_mut(&mut state).unwrap().client = Client::builder(TokioExecutor::new()).build(tls);
    state
        .auth
        .pin(&state.config.db_path, NATIVE_SMOKE_ADMIN_PASSWORD)
        .await
        .unwrap();
    let conn = rusqlite::Connection::open(&state.config.db_path).unwrap();
    let cases = [
        ("claude-sonnet-4-6", Provider::Anthropic),
        ("gpt-5.4", Provider::OpenAI),
        ("kimi-for-coding", Provider::Kimi),
    ];
    for (_, provider) in cases {
        let id = provider.as_str();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1)", [id]).unwrap();
        state.credentials.write().await.insert(
            id.into(),
            Active {
                account_id: id.into(),
                label: id.into(),
                identity: None,
                token: format!("TEST-{id}-UPSTREAM"),
                expires_at_ms: 0,
                provider,
                base_url: Some(upstream.clone()),
                openai_chatgpt: false,
                chatgpt_account_id: None,
            },
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router = format!("http://{}", listener.local_addr().unwrap());
    let app = crate::app(state);
    let (token_id, token) =
        native_client_token(&app, &lab.state.config.db_path, "OpenCode smoke").await;
    let server_app = app.clone();
    let router_server = tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let config = json!({"$schema":"https://opencode.ai/config.json","autoupdate":false,"share":"disabled",
        "model":"agent-router/claude-sonnet-4-6","small_model":"agent-router/claude-sonnet-4-6","enabled_providers":["agent-router"],
        "permission":{"*":"deny","read":"allow"},"agent":{"title":{"disable":true},"summary":{"disable":true},"compaction":{"disable":true}},
        "provider":{"agent-router":{"npm":"@ai-sdk/anthropic","name":"Agent Router",
            "options":{"baseURL":format!("{router}/v1"),"apiKey":"{env:AGENT_ROUTER_API_KEY}"},
            "models":cases.iter().map(|(model,_)| ((*model).to_owned(),json!({"name":model,"tool_call":true,"limit":{"context":128000,"output":4096}}))).collect::<serde_json::Map<String,Value>>()}}});
    let config_path = project.join("opencode.json");
    std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let command = || {
        let mut cmd = tokio::process::Command::new(&cli);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .current_dir(&project)
            .kill_on_drop(true);
        cmd.env("HOME", scratch.join("home"))
            .env("TMPDIR", scratch.join("tmp"))
            .env("LANG", "en_US.UTF-8")
            .env("CI", "1");
        for (key, dir) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
        ] {
            cmd.env(key, scratch.join(dir));
        }
        cmd.env("OPENCODE_CONFIG", &config_path)
            .env("OPENCODE_CONFIG_DIR", scratch.join("config"));
        cmd.env("AGENT_ROUTER_API_KEY", &token);
        for key in [
            "OPENCODE_DISABLE_AUTOUPDATE",
            "OPENCODE_DISABLE_MODELS_FETCH",
            "OPENCODE_DISABLE_DEFAULT_PLUGINS",
            "OPENCODE_DISABLE_LSP_DOWNLOAD",
            "OPENCODE_DISABLE_CLAUDE_CODE",
            "OPENCODE_DISABLE_AUTOCOMPACT",
            "OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER",
        ] {
            cmd.env(key, "true");
        }
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            cmd.env(key, &upstream);
        }
        cmd.env("NO_PROXY", "127.0.0.1,localhost");
        cmd
    };
    let version = command().arg("--version").output().await.unwrap();
    assert!(version.status.success());
    println!(
        "OpenCode {}",
        String::from_utf8_lossy(&version.stdout).trim()
    );
    for (model, provider) in cases {
        let output = tokio::time::timeout(
            Duration::from_secs(60),
            command()
                .args([
                    "--pure",
                    "run",
                    "--model",
                    &format!("agent-router/{model}"),
                    "--title",
                    "Agent Router local smoke",
                    "--format",
                    "json",
                    "Read fixture.txt and confirm its marker.",
                ])
                .output(),
        )
        .await
        .expect("OpenCode timed out")
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{model}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains(&format!("SMOKE_OK {model}")),
            "{model}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<Value> = stdout
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert!(
            events.iter().any(|ev| ev["type"] == "tool_use"
                && ev["part"]["state"]["status"] == "completed"
                && ev["part"]["state"]["output"]
                    .as_str()
                    .is_some_and(|text| text.contains("LOCAL_TOOL_FIXTURE_OK"))),
            "{model}: read did not succeed"
        );
        let session = events[0]["sessionID"].as_str().unwrap();
        for turn in 0..2 {
            let (path, headers, body) =
                tokio::time::timeout(Duration::from_secs(5), requests.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                path,
                if provider == Provider::OpenAI {
                    "/v1/responses"
                } else {
                    "/v1/messages"
                }
            );
            assert_eq!(body["model"], model);
            assert_eq!(headers["x-opencode-session-id"], session);
            assert_eq!(headers["x-session-affinity"], session);
            assert_eq!(headers["x-session-id"], session);
            let ua = headers["user-agent"].to_str().unwrap();
            assert_eq!(crate::analyze::client_of(ua), Some("opencode"));
            let upstream_token = format!("TEST-{}-UPSTREAM", provider.as_str());
            if provider == Provider::Kimi {
                assert_eq!(headers["x-api-key"], upstream_token);
                assert!(!headers.contains_key("authorization"));
            } else {
                assert_eq!(headers["authorization"], format!("Bearer {upstream_token}"));
                assert!(!headers.contains_key("x-api-key"));
            }
            assert!(!headers.contains_key("x-agent-router-token"));
            assert!(!headers.contains_key("x-agent-router-password"));
            assert!(headers
                .values()
                .all(|value| value.to_str().is_ok_and(|value| !value.contains(&token))));
            assert!(!body.to_string().contains(&token));
            if turn == 1 {
                assert!(
                    body.to_string().contains("LOCAL_TOOL_FIXTURE_OK"),
                    "{model}: tool result was not replayed"
                );
            }
            let ev = tokio::time::timeout(Duration::from_secs(5), captured.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ev.status, 200);
            assert_eq!(ev.provider, provider);
            assert_eq!(ev.account_id, provider.as_str());
            assert_eq!(
                ev.session_id.as_deref(),
                Some(format!("opencode:{session}").as_str())
            );
            assert_eq!(
                ev.shape.as_ref().and_then(|shape| shape.cwd.as_deref()),
                project.to_str()
            );
            assert_eq!(
                crate::analyze::client_of(ev.entrypoint.as_deref().unwrap()),
                Some("opencode")
            );
            assert_eq!(ev.usage.input_tokens, 42);
            assert_eq!(ev.usage.output_tokens, 13);
        }
        println!("{model}: {} routed, streamed, read tool completed and replayed; session/cwd/client captured", provider.as_str());
    }
    native_client_revoke(&app, &token_id, &token, "claude-sonnet-4-6", "messages").await;
    assert!(matches!(
        crate::auth::tokens::check(&lab.state.config.db_path, &token).await,
        Err(crate::auth::tokens::Invalid::Revoked)
    ));
    println!("OpenCode Client token: administration denied, inference revoked, no router secret forwarded");
    assert!(
        requests.try_recv().is_err(),
        "unexpected additional model request"
    );
    assert!(
        captured.try_recv().is_err(),
        "unexpected additional capture"
    );
    router_server.abort();
    fake_server.abort();
}

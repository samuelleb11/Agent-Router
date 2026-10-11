// Included only by proxy::tests. Requires an installed Claude Code CLI; its home,
// configuration, read fixture and all upstream services are temporary and local.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Claude Code; run scripts/check-claude-routing.sh"]
async fn the_real_claude_cli_routes_three_providers() {
    use axum::routing::post;
    use std::time::Duration;

    let cli = std::env::var_os("AR_CLAUDE_BIN")
        .expect("set AR_CLAUDE_BIN via scripts/check-claude-routing.sh");
    let lab = Lab::local("claude-routing");
    let scratch = lab
        .state
        .config
        .db_path
        .parent()
        .unwrap()
        .join("claude-cli");
    for dir in [
        "home", "claude", "config", "state", "cache", "data", "tmp", "project",
    ] {
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
            let output =
                native_tool_mock_stream(&body, responses, "Read", json!({"file_path":fixture}));
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
    let fake = axum::Router::new()
        .route("/v1/messages", post(handler.clone()))
        .route("/v1/responses", post(handler))
        .route(
            "/v1/messages/count_tokens",
            post(|| async { axum::Json(json!({"input_tokens":42})) }),
        )
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
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1)",[id]).unwrap();
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
        native_client_token(&app, &lab.state.config.db_path, "Claude Code smoke").await;
    let server_app = app.clone();
    let router_server = tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let command = || {
        let mut cmd = tokio::process::Command::new(&cli);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .current_dir(&project)
            .kill_on_drop(true);
        cmd.env("HOME", scratch.join("home"))
            .env("CLAUDE_CONFIG_DIR", scratch.join("claude"))
            .env("TMPDIR", scratch.join("tmp"))
            .env("LANG", "en_US.UTF-8")
            .env("CI", "1")
            .env("ANTHROPIC_BASE_URL", &router)
            .env("ANTHROPIC_AUTH_TOKEN", &token);
        for (key, dir) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
        ] {
            cmd.env(key, scratch.join(dir));
        }
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            cmd.env(key, &upstream);
        }
        cmd.env("NO_PROXY", "127.0.0.1,localhost")
            .env("DISABLE_TELEMETRY", "1")
            .env("DISABLE_ERROR_REPORTING", "1")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
            .env("CLAUDE_CODE_DISABLE_AUTO_COMPACT", "1");
        cmd
    };
    let version = command().arg("--version").output().await.unwrap();
    assert!(version.status.success());
    println!(
        "Claude Code {}",
        String::from_utf8_lossy(&version.stdout).trim()
    );
    let system = format!(
        "Use Read to inspect the fixture.\nWorking directory: {}",
        project.display()
    );
    for (model, provider) in cases {
        let output = tokio::time::timeout(
            Duration::from_secs(60),
            command()
                .args([
                    "--print",
                    "--verbose",
                    "--output-format",
                    "stream-json",
                    "--safe-mode",
                    "--restricted",
                    "--strict-mcp-config",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                    "--tools",
                    "Read",
                    "--allowedTools",
                    "Read",
                    "--permission-mode",
                    "dontAsk",
                    "--no-session-persistence",
                    "--model",
                    model,
                    "--system-prompt",
                    &system,
                    "Read fixture.txt and confirm its marker.",
                ])
                .output(),
        )
        .await
        .expect("Claude Code timed out")
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{model}: {stdout}\n{stderr}");
        assert!(
            stdout.contains(&format!("SMOKE_OK {model}")),
            "{model}: {stdout}\n{stderr}"
        );
        let events: Vec<Value> = stdout
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert!(
            events.iter().any(|e| e["type"] == "user"
                && e["message"]["content"]
                    .to_string()
                    .contains("LOCAL_TOOL_FIXTURE_OK")),
            "{model}: Read did not execute: {stdout}\n{stderr}"
        );
        assert!(
            events
                .iter()
                .any(|e| e["type"] == "result" && e["is_error"] == false),
            "{model}: native result was not successful"
        );
        let session = events
            .iter()
            .find_map(|e| e["session_id"].as_str())
            .unwrap();
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
            assert_eq!(
                crate::analyze::client_of(headers["user-agent"].to_str().unwrap()),
                Some("claude-code-cli")
            );
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
                .all(|v| v.to_str().is_ok_and(|v| !v.contains(&token))));
            assert!(!body.to_string().contains(&token));
            if turn == 1 {
                assert!(
                    body.to_string().contains("LOCAL_TOOL_FIXTURE_OK"),
                    "{model}: Read output was not replayed"
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
                Some(session),
                "native session header: {:?}; metadata: {}",
                header_session_of(&headers),
                body["metadata"]
            );
            assert_eq!(
                ev.shape.as_ref().and_then(|shape| shape.cwd.as_deref()),
                project.to_str()
            );
            assert_eq!(
                crate::analyze::client_of(ev.entrypoint.as_deref().unwrap()),
                Some("claude-code-cli")
            );
            assert_eq!(ev.usage.input_tokens, 42);
            assert_eq!(ev.usage.output_tokens, 13);
        }
        println!("{model}: {} routed, streamed, Read completed and replayed; native session/client captured",provider.as_str());
    }
    native_client_revoke(&app, &token_id, &token, "claude-sonnet-4-6", "messages").await;
    println!("Claude Code Client token: administration denied, inference revoked, no router secret forwarded");
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

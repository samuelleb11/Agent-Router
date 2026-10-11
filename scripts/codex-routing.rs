// Included only by proxy::tests. The smoke runs the installed Codex executable
// with a temporary CODEX_HOME and workspace against three local fake providers.

fn codex_mock_stream(body: &Value, responses: bool) -> String {
    let model = body["model"].as_str().unwrap();
    let answered = if responses {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call_output")
    } else {
        body["messages"].as_array().unwrap().iter().any(|message| {
            message["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"))
        })
    };
    let tools = body["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|tool| {
            tool["name"]
                .as_str()
                .is_some_and(|name| name.ends_with("exec_command"))
        })
        .expect("Codex CLI must advertise its command tool");
    let name = tool["name"].as_str().unwrap();
    let args =
        json!({"cmd":"cat fixture.txt","yield_time_ms":1000,"max_output_tokens":100}).to_string();
    let answer = format!("SMOKE_OK {model}");
    let frames = if responses {
        let item = if answered {
            json!({"type":"message","id":"msg_codex_smoke","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":answer,"annotations":[]}]})
        } else {
            json!({"type":"function_call","id":"fc_codex_smoke","call_id":"call_codex_smoke","name":name,"arguments":args,"status":"completed"})
        };
        let response = json!({"id":"resp_codex_smoke","object":"response","status":"completed","model":model,
            "output":[item],"usage":{"input_tokens":42,"output_tokens":13,"total_tokens":55}});
        let mut frames = vec![
            json!({"type":"response.created","response":{"id":"resp_codex_smoke","model":model,"status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"item":if answered {
                json!({"type":"message","id":"msg_codex_smoke","role":"assistant","status":"in_progress","content":[]})
            } else { json!({"type":"function_call","id":"fc_codex_smoke","call_id":"call_codex_smoke","name":name,"arguments":"","status":"in_progress"}) }}),
        ];
        if answered {
            frames.push(json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_codex_smoke","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
            frames.push(json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_codex_smoke","content_index":0,"delta":answer}));
            frames.push(json!({"type":"response.output_text.done","output_index":0,"item_id":"msg_codex_smoke","content_index":0,"text":answer}));
            frames.push(json!({"type":"response.content_part.done","output_index":0,"item_id":"msg_codex_smoke","content_index":0,"part":item["content"][0]}));
        } else {
            frames.push(json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_codex_smoke","delta":args}));
            frames.push(json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_codex_smoke","arguments":args}));
        }
        frames.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
        frames.push(json!({"type":"response.completed","response":response}));
        frames
    } else {
        vec![
            json!({"type":"message_start","message":{"id":"msg_codex_smoke","type":"message","role":"assistant","model":model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":42,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":if answered {json!({"type":"text","text":""})} else {json!({"type":"tool_use","id":"call_codex_smoke","name":name,"input":{}})}}),
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
#[ignore = "requires version-matched Codex; run scripts/check-primary-codex.sh catalogue-core"]
async fn native_codex_catalogue_additions_and_removals() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use std::time::Duration;
    let cli=std::env::var_os("AR_CODEX_BIN").expect("use the catalogue qualification script");
    let version=tokio::process::Command::new(&cli).arg("--version").output().await.unwrap();
    assert!(version.status.success());
    assert_eq!(String::from_utf8_lossy(&version.stdout).trim(),format!("codex-cli {}",crate::codex::catalogue::QUALIFIED_VERSION));
    let lab=Lab::local("native-codex-catalogue");
    lab.state.auth.pin(&lab.state.config.db_path,NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
    let home=lab.state.config.db_path.parent().unwrap().join("native-home");
    let codex=home.join(".codex");std::fs::create_dir_all(&codex).unwrap();
    let config=codex.join("config.toml");
    let conn=rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
    for provider in [Provider::Anthropic,Provider::OpenAI,Provider::Kimi] {
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1760000000000)",[provider.as_str()]).unwrap();
        let route=crate::provider_registry::ModelRoute {
            alias:format!("team-{}",provider.as_str()),service:provider.as_str().into(),upstream_model:"fixture".into(),
            account_id:provider.as_str().into(),display_name:Some(format!("Team {}",provider.name())),
            capabilities:Some(crate::provider_registry::Capabilities {images:false,reasoning:false,context_tokens:Some(64000),output_tokens:Some(2000),..Default::default()}),
        };
        conn.execute("INSERT INTO provider_model_routes(alias,definition) VALUES(?1,?2)",rusqlite::params![route.alias,serde_json::to_string(&route).unwrap()]).unwrap();
    }
    let (_,token)=native_client_token(&lab.app,&lab.state.config.db_path,"Model catalogue").await;
    async fn metadata(app:&axum::Router,token:&str)->crate::models::ClientCatalogue {
        let response=native_client_request(app,"GET","/_router/models",Some(token),Value::Null).await;
        assert_eq!(response.status(),StatusCode::OK,"Client token must read model metadata");
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1<<20).await.unwrap()).unwrap()
    }
    crate::codex::apply(&config,&home.join("provider-kept.json"),"http://127.0.0.1:1",&token,true).unwrap();
    let first=metadata(&lab.app,&token).await;
    crate::codex::catalogue::sync(&config,&first,true).unwrap();
    async fn picker(cli:&std::ffi::OsStr,home:&std::path::Path,codex:&std::path::Path)->Value {
        let mut child=tokio::process::Command::new(cli).args(["app-server","--listen","stdio://"])
            .env_clear().env("PATH",std::env::var_os("PATH").unwrap_or_default()).env("HOME",home).env("CODEX_HOME",codex)
            .env("TMPDIR",home).env("XDG_CONFIG_HOME",home.join("config")).env("XDG_CACHE_HOME",home.join("cache"))
            .env("RUST_LOG","off").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .kill_on_drop(true).spawn().unwrap();
        let mut input=child.stdin.take().unwrap();
        let mut lines=BufReader::new(child.stdout.take().unwrap()).lines();
        let initialize=json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"agent-router-catalogue","version":"1"},"capabilities":{"experimentalApi":true}}});
        input.write_all(format!("{initialize}\n").as_bytes()).await.unwrap();
        async fn answer(lines:&mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,id:u32)->Value {
            tokio::time::timeout(Duration::from_secs(20),async {
                while let Some(line)=lines.next_line().await.unwrap() {
                    let value:Value=serde_json::from_str(&line).unwrap();
                    if value["id"]==id {assert!(value.get("error").is_none(),"{value}");return value["result"].clone();}
                }
                panic!("native app-server closed before replying");
            }).await.expect("native app-server model/list timed out")
        }
        answer(&mut lines,1).await;
        input.write_all(b"{\"method\":\"initialized\"}\n").await.unwrap();
        let query=json!({"id":2,"method":"model/list","params":{"limit":100,"includeHidden":true}});
        input.write_all(format!("{query}\n").as_bytes()).await.unwrap();
        let result=answer(&mut lines,2).await;
        child.kill().await.unwrap();child.wait().await.unwrap();
        result
    }
    let listed=picker(&cli,&home,&codex).await;
    let rows=listed["data"].as_array().expect("native model/list data");
    assert_eq!(rows.len(),first.models.len(),"router catalogue must replace bundled models: {listed}");
    for provider in [Provider::Anthropic,Provider::OpenAI,Provider::Kimi] {
        let row=rows.iter().find(|m|m["model"]==format!("team-{}",provider.as_str())).unwrap();
        assert_eq!(row["displayName"],format!("Team {}",provider.name()));
        assert_eq!(row["supportedReasoningEfforts"],json!([]));
        assert_eq!(row["inputModalities"],json!(["text"]));
    }
    conn.execute("DELETE FROM provider_model_routes WHERE alias='team-anthropic'",[]).unwrap();
    let mut next=crate::provider_registry::model_route(&conn,"team-openai").unwrap().unwrap();
    next.alias="team-new".into();next.display_name=Some("New server model".into());
    conn.execute("INSERT INTO provider_model_routes(alias,definition) VALUES(?1,?2)",rusqlite::params![next.alias,serde_json::to_string(&next).unwrap()]).unwrap();
    let updated=metadata(&lab.app,&token).await;
    assert_eq!(crate::codex::catalogue::sync(&config,&updated,false).unwrap(),Some(true));
    let listed=picker(&cli,&home,&codex).await;
    let rows=listed["data"].as_array().unwrap();
    assert_eq!(rows.len(),updated.models.len());
    assert!(!rows.iter().any(|m|m["model"]=="team-anthropic"));
    assert!(rows.iter().any(|m|m["model"]=="team-new" && m["displayName"]=="New server model"));
    assert_eq!(crate::codex::catalogue::sync(&config,&updated,false).unwrap(),Some(false));
    assert!(!std::fs::read_to_string(codex.join(crate::codex::catalogue::FILE)).unwrap().contains(&token));
    crate::codex::catalogue::restore(&config).unwrap();
    crate::codex::remove(&config,&home.join("provider-kept.json")).unwrap();
    println!("NATIVE_CATALOGUE_OK: three provider aliases, metadata, additions/removals after explicit restart, private reversible config");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Codex; run scripts/check-codex-routing.sh"]
async fn the_real_codex_cli_routes_three_providers() {
    use axum::routing::post;
    use std::time::Duration;

    let cli = std::env::var_os("AR_CODEX_BIN")
        .expect("set AR_CODEX_BIN via scripts/check-codex-routing.sh");
    let lab = Lab::local("codex-routing");
    let scratch = lab.state.config.db_path.parent().unwrap().join("codex-cli");
    for dir in [
        "home", "codex", "config", "state", "cache", "data", "tmp", "project",
    ] {
        std::fs::create_dir_all(scratch.join(dir)).unwrap();
    }
    let project = scratch.join("project").canonicalize().unwrap();
    std::fs::write(project.join("fixture.txt"), "LOCAL_TOOL_FIXTURE_OK\n").unwrap();
    let (observed, mut requests) = tokio::sync::mpsc::channel(16);
    let handler = move |req: Request<Body>| {
        let observed = observed.clone();
        async move {
            let (parts, body) = req.into_parts();
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(body, MAX_REQUEST_BODY).await.unwrap(),
            )
            .unwrap();
            let output = codex_mock_stream(&body, parts.uri.path() == "/v1/responses");
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
    crate::pricing::ensure_table(&conn).unwrap();
    for (model, provider) in cases {
        let id = provider.as_str();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1)", [id]).unwrap();
        conn.execute("INSERT INTO model_rate(model,key) VALUES(?1,NULL)",[model]).unwrap();
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
        native_client_token(&app, &lab.state.config.db_path, "Codex smoke").await;
    let server_app = app.clone();
    let router_server = tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    crate::codex::apply(
        &scratch.join("codex/config.toml"),
        &scratch.join("kept.json"),
        &router,
        &token,
        true,
    )
    .unwrap();
    crate::codex::catalogue::sync(&scratch.join("codex/config.toml"),&crate::models::client_catalogue(&conn).unwrap(),true).unwrap();
    let command = || {
        let mut cmd = tokio::process::Command::new(&cli);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .current_dir(&project)
            .kill_on_drop(true);
        cmd.env("HOME", scratch.join("home"))
            .env("CODEX_HOME", scratch.join("codex"))
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
        // Trap optional outbound requests; the native client and its command tool
        // need only the loopback router, loopback fake providers and fixture file.
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            cmd.env(key, &upstream);
        }
        cmd.env("NO_PROXY", "127.0.0.1,localhost");
        cmd
    };
    let version = command().arg("--version").output().await.unwrap();
    assert!(version.status.success());
    println!("{}", String::from_utf8_lossy(&version.stdout).trim());
    for (model, provider) in cases {
        let output = tokio::time::timeout(
            Duration::from_secs(60),
            command()
                .args([
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "--color",
                    "never",
                    "--sandbox",
                    "read-only",
                    "-c",
                    "approval_policy=\"never\"",
                    "-c",
                    "features.multi_agent=false",
                    "--model",
                    model,
                    "Read fixture.txt with a command and confirm its marker.",
                ])
                .output(),
        )
        .await
        .expect("Codex timed out")
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
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert!(
            events.iter().any(|event| event["type"] == "item.completed"
                && event["item"]["type"] == "command_execution"
                && event["item"]["exit_code"] == 0
                && event["item"]["aggregated_output"]
                    .as_str()
                    .is_some_and(|text| text.contains("LOCAL_TOOL_FIXTURE_OK"))),
            "{model}: command tool did not read fixture: {stdout}\n{stderr}"
        );
        let session = events
            .iter()
            .find(|event| event["type"] == "thread.started")
            .unwrap()["thread_id"]
            .as_str()
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
                header_session_of(&headers).as_deref(),
                Some(format!("codex:{session}").as_str())
            );
            let ua = headers["user-agent"].to_str().unwrap();
            assert_eq!(
                crate::analyze::client_of(ua),
                Some("codex-cli"),
                "native Codex user-agent: {ua}"
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
                .all(|value| value.to_str().is_ok_and(|value| !value.contains(&token))));
            assert!(!body.to_string().contains(&token));
            if turn == 1 {
                assert!(
                    body.to_string().contains("LOCAL_TOOL_FIXTURE_OK"),
                    "{model}: command result was not replayed"
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
                Some(format!("codex:{session}").as_str())
            );
            assert_eq!(
                ev.shape.as_ref().and_then(|shape| shape.cwd.as_deref()),
                project.to_str()
            );
            assert_eq!(
                crate::analyze::client_of(ev.entrypoint.as_deref().unwrap()),
                Some("codex-cli")
            );
            assert_eq!(ev.usage.input_tokens, 42);
            assert_eq!(ev.usage.output_tokens, 13);
        }
        println!("{model}: {} routed, streamed, command tool completed and replayed; session/cwd/client captured", provider.as_str());
    }
    if let Some(driver)=std::env::var_os("AR_CODEX_SIDEBAR_DRIVER") {
        let output=tokio::time::timeout(Duration::from_secs(240),tokio::process::Command::new("node")
            .arg(driver).env("AR_SIDEBAR_SCRATCH",&scratch).env("AR_SIDEBAR_ROUTER",&router)
            .env("AR_SIDEBAR_TOKEN",&token).env("AR_SIDEBAR_ADMIN",NATIVE_SMOKE_ADMIN_PASSWORD)
            .env("AR_SIDEBAR_MODELS",serde_json::to_string(&cases.iter().map(|(m,p)|json!({"model":m,"provider":p.as_str()})).collect::<Vec<_>>()).unwrap())
            .kill_on_drop(true).output()).await.expect("native sidebar acceptance timed out").unwrap();
        let stdout=String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(),"native sidebar failed:\n{stdout}\n{}",String::from_utf8_lossy(&output.stderr));
        assert!(stdout.contains("NATIVE_SIDEBAR_OK models=3 added=1 removed=1"),"sidebar cases must execute: {stdout}");
        println!("{stdout}");
        // Actual native sidebar prompts must complete a local command and replay
        // its result through each provider, in addition to model-picker proof.
        for (model,provider) in cases {
            for turn in 0..2 {
                let (path,headers,body)=tokio::time::timeout(Duration::from_secs(5),requests.recv()).await.unwrap().unwrap();
                assert_eq!(body["model"],model);
                assert_eq!(path,if provider==Provider::OpenAI {"/v1/responses"} else {"/v1/messages"});
                assert!(headers.values().all(|h|h.to_str().is_ok_and(|h|!h.contains(&token))));
                if turn==1 {assert!(body.to_string().contains("LOCAL_TOOL_FIXTURE_OK"));}
                let event=tokio::time::timeout(Duration::from_secs(5),captured.recv()).await.unwrap().unwrap();
                assert_eq!(event.status,200);assert_eq!(event.provider,provider);
                assert_eq!(crate::analyze::client_of(event.entrypoint.as_deref().unwrap()),Some("codex-vscode"));
            }
        }
    }
    native_client_revoke(&app, &token_id, &token, "claude-sonnet-4-6", "responses").await;
    assert!(matches!(
        crate::auth::tokens::check(&lab.state.config.db_path, &token).await,
        Err(crate::auth::tokens::Invalid::Revoked)
    ));
    println!(
        "Codex Client token: administration denied, inference revoked, no router secret forwarded"
    );
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

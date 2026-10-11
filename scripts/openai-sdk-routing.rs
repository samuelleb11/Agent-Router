// An actual official SDK, speaking HTTP to the router and three local providers.
// Included in proxy::tests; no native client or real provider credential is needed.

fn sdk_fixture(body: &Value, responses: bool) -> String {
    let model = body["model"].as_str().unwrap();
    let serialized = body.to_string();
    let tool = serialized.contains("TOOL_REQUEST") && !serialized.contains("SDK_TOOL_RESULT");
    let structured = serialized.contains("json_schema");
    let text = if structured { "{\"ok\":true}" } else { "SDK_OK" };
    let args = "{\"path\":\"fixture\"}";
    let usage = json!({"input_tokens":37,"output_tokens":11});
    let item = if tool {
        json!({"id":"fc_sdk","type":"function_call","call_id":"call_sdk","name":"read","arguments":args,"status":"completed"})
    } else {
        json!({"id":"msg_sdk","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})
    };
    let response = json!({"id":"resp_sdk","object":"response","model":model,"status":"completed","output":[item],
        "usage":{"input_tokens":37,"input_tokens_details":{"cached_tokens":0},"output_tokens":11,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":48}});
    let message = json!({"id":"msg_sdk","type":"message","role":"assistant","model":model,
        "content":[if tool { json!({"type":"tool_use","id":"call_sdk","name":"read","input":{"path":"fixture"}}) } else { json!({"type":"text","text":text}) }],
        "stop_reason":if tool {"tool_use"} else {"end_turn"},"stop_sequence":null,"usage":usage});
    if body["stream"] != true {
        return if responses {response} else {message}.to_string();
    }
    let frames = if responses {
        let mut frames = vec![
            json!({"type":"response.created","response":{"id":"resp_sdk","object":"response","model":model,"status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"item":if tool {
                json!({"id":"fc_sdk","type":"function_call","call_id":"call_sdk","name":"read","arguments":"","status":"in_progress"})
            } else {json!({"id":"msg_sdk","type":"message","role":"assistant","content":[],"status":"in_progress"})}}),
        ];
        if tool {
            frames.push(json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_sdk","delta":args}));
            frames.push(json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_sdk","arguments":args}));
        } else {
            frames.push(json!({"type":"response.content_part.added","output_index":0,"content_index":0,"item_id":"msg_sdk","part":{"type":"output_text","text":"","annotations":[]}}));
            frames.push(json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"msg_sdk","delta":text}));
            frames.push(json!({"type":"response.output_text.done","output_index":0,"content_index":0,"item_id":"msg_sdk","text":text}));
            frames.push(json!({"type":"response.content_part.done","output_index":0,"content_index":0,"item_id":"msg_sdk","part":item["content"][0]}));
        }
        frames.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
        frames.push(json!({"type":"response.completed","response":response}));
        frames
    } else {
        vec![
            json!({"type":"message_start","message":{"id":"msg_sdk","type":"message","role":"assistant","model":model,"content":[],"usage":{"input_tokens":37,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":if tool {json!({"type":"tool_use","id":"call_sdk","name":"read","input":{}})} else {json!({"type":"text","text":""})}}),
            json!({"type":"content_block_delta","index":0,"delta":if tool {json!({"type":"input_json_delta","partial_json":args})} else {json!({"type":"text_delta","text":text})}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":if tool {"tool_use"} else {"end_turn"},"stop_sequence":null},"usage":{"output_tokens":11}}),
            json!({"type":"message_stop"}),
        ]
    };
    frames.iter().map(|f|format!("event: {}\ndata: {f}\n\n", f["type"].as_str().unwrap())).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires official Python SDK; run scripts/check-primary-codex.sh gateway"]
async fn official_openai_sdk_routes_three_providers() {
    use axum::routing::post;
    use std::time::Duration;

    let python = std::env::var_os("AR_SDK_PYTHON").expect("use the qualification script");
    let lab = Lab::local("official-sdk");
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = observed.clone();
    let handler = move |req: Request<Body>| {
        let observed = observed.clone();
        async move {
            let (parts, body) = req.into_parts();
            let body: Value = serde_json::from_slice(&axum::body::to_bytes(body, MAX_REQUEST_BODY).await.unwrap()).unwrap();
            let error = body.to_string().contains("UPSTREAM_ERROR");
            let responses = parts.uri.path() == "/v1/responses";
            let output = if error {json!({"error":{"type":"rate_limit_error","message":"Fixture capacity exhausted"}}).to_string()} else {sdk_fixture(&body, responses)};
            let content_type = if body["stream"] == true && !error {"text/event-stream"} else {"application/json"};
            observed.lock().unwrap().push((parts.uri.path().to_owned(), parts.headers, body));
            let chunks: Vec<_> = output.as_bytes().chunks(13).map(|b|Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(b))).collect();
            Response::builder().status(if error {429} else {200}).header("content-type",content_type)
                .body(Body::from_stream(futures_util::stream::iter(chunks))).unwrap()
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let fake = axum::Router::new().route("/v1/messages", post(handler.clone())).route("/v1/responses", post(handler));
    let fake_server = tokio::spawn(async move {axum::serve(listener, fake).await.unwrap()});
    let config = Config {
        server:true,bind:([127,0,0,1],0).into(),upstream:upstream.clone(),db_path:lab.state.config.db_path.clone(),
        body_cap:65536,dump_headers:0,allow_hosts:vec![],public_url:None,push_keys_dir:None,agents_dir:None,
        drain:Duration::from_secs(1),stop_delay:Duration::ZERO,
    };
    let (capture, mut captured) = tokio::sync::mpsc::channel(256);
    let (network, _) = crate::network::switch();
    let mut state = AppState::new(config,capture,network).unwrap();
    let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
    Arc::get_mut(&mut state).unwrap().client = Client::builder(TokioExecutor::new()).build(tls);
    state.auth.pin(&state.config.db_path,NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
    let conn = rusqlite::Connection::open(&state.config.db_path).unwrap();
    crate::pricing::ensure_table(&conn).unwrap();
    for (provider,model) in [(Provider::Anthropic,"claude-sdk-test"),(Provider::OpenAI,"gpt-sdk-test"),(Provider::Kimi,"kimi-for-coding")] {
        let id = provider.as_str();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1760000000000)",[id]).unwrap();
        conn.execute("INSERT INTO model_rate(model,key) VALUES(?1,NULL)",[model]).unwrap();
        state.credentials.write().await.insert(id.into(), Active {
            account_id:id.into(),label:id.into(),identity:None,token:format!("SDK-UPSTREAM-{id}"),expires_at_ms:0,
            provider,base_url:Some(upstream.clone()),openai_chatgpt:false,chatgpt_account_id:None,
        });
    }
    let app = crate::app(state);
    let (id,token) = native_client_token(&app,&lab.state.config.db_path,"Official SDK").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router = format!("http://{}/v1",listener.local_addr().unwrap());
    let server_app = app.clone();
    let server = tokio::spawn(async move {axum::serve(listener, server_app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap()});
    async fn run(python:&std::ffi::OsStr, url:&str,token:&str,old:Option<&str>) {
        let mut command = tokio::process::Command::new(python);
        command.arg(concat!(env!("CARGO_MANIFEST_DIR"),"/scripts/openai-sdk-smoke.py"))
            .env("AR_SDK_URL",url).env("AR_SDK_TOKEN",token).env("AR_SDK_OLD_TOKEN",old.unwrap_or_default())
            .env_remove("OPENAI_API_KEY").env_remove("OPENAI_BASE_URL").env("NO_PROXY","127.0.0.1,localhost")
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(90),command.output()).await.expect("SDK timed out").unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "SDK failed:\n{stdout}\n{}",String::from_utf8_lossy(&output.stderr));
        assert!(stdout.contains(if old.is_some() {"SDK_ROTATION_OK"} else {"SDK_MATRIX_OK cases=42"}), "intended SDK cases must execute: {stdout}");
        println!("{stdout}");
    }
    run(&python,&router,&token,None).await;
    let (_,renewed) = native_client_token(&app,&lab.state.config.db_path,"Official SDK replacement").await;
    native_client_revoke(&app,&id,&token,"claude-sdk-test","responses").await;
    run(&python,&router,&renewed,Some(&token)).await;
    let requests = requests.lock().unwrap();
    for provider in [Provider::Anthropic,Provider::OpenAI,Provider::Kimi] {
        let credential = format!("SDK-UPSTREAM-{}",provider.as_str());
        let rows:Vec<_> = requests.iter().filter(|(_,headers,_)|headers.values().any(|h|h.to_str().is_ok_and(|v|v.contains(&credential)))).collect();
        assert!(rows.len()>=14,"{} missing SDK cases",provider.as_str());
        assert!(rows.iter().all(|(path,_,_)|path==if provider==Provider::OpenAI {"/v1/responses"} else {"/v1/messages"}));
        assert!(rows.iter().any(|(_,_,body)|body.to_string().contains("SDK_TOOL_RESULT")),"full tool history missing");
        assert!(rows.iter().any(|(_,_,body)|body.to_string().contains("json_schema")),"structured output missing");
        for (_,headers,body) in rows {
            assert!(!headers.contains_key("x-agent-router-token"));
            assert!(headers.values().all(|v|v.to_str().is_ok_and(|v|!v.contains(&token)&&!v.contains(&renewed))));
            assert!(!body.to_string().contains(&token));
        }
    }
    drop(requests);
    let mut count = 0;
    while let Ok(event)=captured.try_recv() {
        assert!(matches!(event.provider,Provider::Anthropic|Provider::OpenAI|Provider::Kimi));
        if event.status==200 {assert_eq!(event.usage.input_tokens,37);assert_eq!(event.usage.output_tokens,11);}
        count+=1;
    }
    assert!(count>=42,"SDK requests must be captured");
    server.abort();fake_server.abort();
}

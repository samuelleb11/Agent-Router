// Explicit opt-in live acceptance. Login files are read only. Every client gets
// private settings, a read-only fixture and a short-lived Client-scope token.

fn live_json(path: &std::path::Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn live_model(home: &std::path::Path, provider: Provider) -> String {
    let key = format!("AR_LIVE_{}_MODEL", provider.as_str().to_ascii_uppercase());
    if let Ok(model) = std::env::var(key) { return model; }
    match provider {
        Provider::Anthropic => "claude-sonnet-4-6".into(),
        Provider::Kimi => "kimi-for-coding".into(),
        Provider::OpenAI => live_json(&home.join(".codex/models_cache.json"))
            .and_then(|v| v["models"].as_array()?.iter().find_map(|model| model["slug"].as_str().map(str::to_owned)))
            .unwrap_or_else(|| "gpt-5.4".into()),
    }
}

fn live_credentials(home: &std::path::Path, provider: Provider) -> anyhow::Result<Active> {
    let key = match provider { Provider::Anthropic => "ANTHROPIC_API_KEY", Provider::OpenAI => "OPENAI_API_KEY", Provider::Kimi => "KIMI_API_KEY" };
    let mut active = Active { account_id: provider.as_str().into(), label: provider.as_str().into(), identity: None,
        token: String::new(), expires_at_ms: 0, provider, base_url: None, openai_chatgpt: false, chatgpt_account_id: None };
    if let Some(token) = std::env::var(key).ok().filter(|token| !token.trim().is_empty()) {
        active.token = token;
        return Ok(active);
    }
    match provider {
        Provider::Anthropic => {
            let selected = std::env::var("AR_LIVE_ANTHROPIC_ACCOUNT").ok();
            active.token = live_json(&home.join(".agent-router/credentials.json"))
                .and_then(|v| v["accounts"].as_object()?.iter().filter(|(id, _)| selected.as_ref().is_none_or(|selected| *id == selected))
                    .find_map(|(_, entry)| entry["token"].as_str().filter(|t| t.starts_with("sk-ant-")).map(str::to_owned)))
                .ok_or_else(|| anyhow::anyhow!("Anthropic credential unavailable; set ANTHROPIC_API_KEY"))?;
        }
        Provider::OpenAI => {
            let raw = std::fs::read_to_string(home.join(".codex/auth.json"))?;
            let resolved = crate::accounts::parse_codex_credentials(&raw)?;
            active.token = resolved.token;
            active.expires_at_ms = resolved.expires_at_ms;
            active.openai_chatgpt = resolved.openai_chatgpt;
            active.chatgpt_account_id = resolved.chatgpt_account_id;
        }
        Provider::Kimi => {
            // A configured remote router may already hold the Kimi key. Its own
            // authentication is used for this HTTPS Messages hop; no key export.
            let agent = live_json(&home.join(".agent-router/agent.json"))
                .ok_or_else(|| anyhow::anyhow!("Kimi credential unavailable; set KIMI_API_KEY"))?;
            let server = agent["server"].as_str().filter(|s| s.starts_with("https://"))
                .ok_or_else(|| anyhow::anyhow!("Kimi remote router requires HTTPS"))?;
            active.token = agent["password"].as_str().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Kimi remote router credential unavailable"))?.into();
            active.base_url = Some(server.into());
        }
    }
    anyhow::ensure!(active.expires_at_ms == 0 || active.expires_at_ms > crate::db::now_ms(), "{} login expired", provider.as_str());
    Ok(active)
}

fn live_command(cli: &std::ffi::OsStr, root: &std::path::Path, project: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cli);
    cmd.env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default()).current_dir(project).kill_on_drop(true);
    cmd.env("HOME", root.join("home")).env("TMPDIR", root.join("tmp")).env("LANG", "en_US.UTF-8").env("CI", "1");
    for (key, dir) in [("XDG_CONFIG_HOME","config"),("XDG_STATE_HOME","state"),("XDG_CACHE_HOME","cache"),("XDG_DATA_HOME","data")] { cmd.env(key, root.join(dir)); }
    cmd
}

fn live_safe(text: &str, secrets: &[String]) -> String {
    let mut safe = text.to_owned();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) { safe = safe.replace(secret, "<redacted>"); }
    safe.chars().rev().take(800).collect::<String>().chars().rev().collect()
}

#[tokio::test]
async fn chatgpt_headerless_streams_translate_and_capture_usage() {
    use axum::routing::post;
    let lab = Lab::local("chatgpt-headerless");
    let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
    conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES('openai','OpenAI','token','openai',1,1)", []).unwrap();
    let (capture, mut captured) = tokio::sync::mpsc::channel(16);
    let (network, _) = crate::network::switch();
    let response = json!({"id":"resp_headerless","object":"response","model":"gpt-test","status":"completed",
        "output":[{"type":"message","id":"msg_headerless","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Headerless OK","annotations":[]}]}],
        "usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":60},"output_tokens":5}});
    let stream = [
        json!({"type":"response.created","response":{"id":"resp_headerless","model":"gpt-test","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_headerless","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_headerless","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_headerless","content_index":0,"delta":"Headerless OK"}),
        json!({"type":"response.output_item.done","output_index":0,"item":response["output"][0]}),
        json!({"type":"response.completed","response":response}),
    ].iter().map(|frame|format!("event: {}\ndata: {frame}\n\n",frame["type"].as_str().unwrap())).collect::<String>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}",listener.local_addr().unwrap());
    let fake = axum::Router::new().route("/v1/responses",post(move || { let stream=stream.clone(); async move {
        // This reproduces the live backend's absent Content-Type, including its
        // valid response.completed token counts.
        Response::builder().body(Body::from(stream)).unwrap()
    }}));
    let server = tokio::spawn(async move { axum::serve(listener,fake).await.unwrap() });
    let config = Config { server: true,bind:([127,0,0,1],0).into(),upstream:upstream.clone(),db_path:lab.state.config.db_path.clone(),
        body_cap:65536,dump_headers:0,allow_hosts:vec![],public_url:None,push_keys_dir:None,agents_dir:None,
        drain:std::time::Duration::from_secs(1),stop_delay:std::time::Duration::ZERO };
    let mut state = AppState::new(config,capture,network).unwrap();
    let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
    Arc::get_mut(&mut state).unwrap().client=Client::builder(TokioExecutor::new()).build(tls);
    state.auth.pin(&state.config.db_path,NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
    state.credentials.write().await.insert("openai".into(),Active{account_id:"openai".into(),label:"OpenAI".into(),identity:None,
        token:"TEST-UPSTREAM".into(),expires_at_ms:0,provider:Provider::OpenAI,base_url:Some(upstream),openai_chatgpt:true,chatgpt_account_id:None});
    let app=crate::app(state);
    let (_,token)=native_client_token(&app,&lab.state.config.db_path,"Headerless stream regression").await;
    for (path,streaming) in [("responses",true),("messages",true),("messages",false),("chat/completions",true),("chat/completions",false)] {
        let body = match path {
            "responses"=>json!({"model":"gpt-test","input":"Hello","stream":streaming}),
            "messages"=>json!({"model":"gpt-test","messages":[{"role":"user","content":"Hello"}],"max_tokens":32,"stream":streaming}),
            _=>json!({"model":"gpt-test","messages":[{"role":"user","content":"Hello"}],"stream":streaming}),
        };
        let response=native_client_request(&app,"POST",&format!("/v1/{path}"),Some(&token),body).await;
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(response.headers()["content-type"],if streaming {"text/event-stream"}else{"application/json"});
        let bytes=axum::body::to_bytes(response.into_body(),65536).await.unwrap();
        let text=String::from_utf8_lossy(&bytes);
        assert!(text.contains("Headerless OK"),"{path}: {text}");
        let event=captured.recv().await.unwrap();
        assert_eq!(event.usage.input_tokens,40);
        assert_eq!(event.usage.cache_read_tokens,60);
        assert_eq!(event.usage.output_tokens,5);
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real provider inference; run scripts/check-live-client-routing.sh"]
async fn native_clients_complete_live_provider_acceptance() {
    use std::time::Duration;
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME required for read-only login discovery"));
    let lab = Lab::local("live-client-routing");
    let root = lab.state.config.db_path.parent().unwrap().join("live-clients");
    for dir in ["home","config","state","cache","data","tmp","project","codex"] { std::fs::create_dir_all(root.join(dir)).unwrap(); }
    let project = root.join("project").canonicalize().unwrap();
    let marker = format!("AR_LIVE_{}_{}", std::process::id(), crate::db::now_ms());
    std::fs::write(project.join("fixture.txt"), &marker).unwrap();
    let config = Config { server: true, bind: ([127,0,0,1],0).into(), upstream: "https://api.anthropic.com".into(),
        db_path: lab.state.config.db_path.clone(), body_cap: 65536, dump_headers: 0, allow_hosts: vec![],
        public_url: None, push_keys_dir: None, agents_dir: None, drain: Duration::from_secs(1), stop_delay: Duration::ZERO };
    let (capture, mut captured) = tokio::sync::mpsc::channel(128);
    let (network, _) = crate::network::switch();
    let state = AppState::new(config, capture, network).unwrap();
    state.auth.pin(&state.config.db_path, NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
    let mut secrets = vec![];
    let mut cases = vec![];
    let mut failures = vec![];
    let conn = rusqlite::Connection::open(&state.config.db_path).unwrap();
    for provider in [Provider::Anthropic,Provider::OpenAI,Provider::Kimi] {
        match live_credentials(&home, provider) {
            Ok(active) => {
                println!("{} credential: {}", provider.as_str(), if active.openai_chatgpt {"Codex login"} else if active.base_url.is_some() {"configured HTTPS router"} else {"provider token"});
                secrets.push(active.token.clone());
                conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1)", [provider.as_str()]).unwrap();
                state.credentials.write().await.insert(provider.as_str().into(), active);
                cases.push((provider, live_model(&home, provider)));
            }
            Err(error) => failures.push(format!("{} prerequisite: {error}", provider.as_str())),
        }
    }
    let app = crate::app(state);
    let (token_id, token) = native_client_token(&app, &lab.state.config.db_path, "Live native acceptance").await;
    secrets.push(token.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router = format!("http://{}", listener.local_addr().unwrap());
    let served = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
    crate::codex::apply(&root.join("codex/config.toml"), &root.join("kept.json"), &router, &token, true).unwrap();
    let opencode = json!({"$schema":"https://opencode.ai/config.json","autoupdate":false,"share":"disabled",
        "enabled_providers":["agent-router"],"permission":{"*":"deny","read":"allow"},
        "agent":{"title":{"disable":true},"summary":{"disable":true},"compaction":{"disable":true}},
        "provider":{"agent-router":{"npm":"@ai-sdk/anthropic","name":"Agent Router",
            "options":{"baseURL":format!("{router}/v1"),"apiKey":"{env:AGENT_ROUTER_API_KEY}"},
            "models":cases.iter().map(|(_,model)| (model.clone(),json!({"name":model,"tool_call":true,"limit":{"context":128000,"output":4096}}))).collect::<serde_json::Map<String,Value>>()}}});
    let opencode_config = project.join("opencode.json");
    std::fs::write(&opencode_config, serde_json::to_vec_pretty(&opencode).unwrap()).unwrap();
    let prompt = "Read fixture.txt using your file or command tool. Reply with the exact marker from that file and no other text.";
    for (provider, model) in cases {
        for client in ["CLAUDE","CODEX","OPENCODE"] {
            let cli = std::env::var_os(format!("AR_LIVE_{client}_BIN")).expect("run the live acceptance script");
            let mut command = live_command(&cli, &root, &project);
            match client {
                "CLAUDE" => {
                    command.env("ANTHROPIC_BASE_URL", &router).env("ANTHROPIC_AUTH_TOKEN", &token)
                        .env("CLAUDE_CONFIG_DIR",root.join("config/claude")).env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC","1")
                        .env("DISABLE_AUTOUPDATER","1").env("DISABLE_TELEMETRY","1");
                    command.args(["--print","--verbose","--output-format","stream-json","--safe-mode","--restricted",
                        "--strict-mcp-config","--mcp-config","{\"mcpServers\":{}}","--tools","Read","--allowedTools","Read",
                        "--permission-mode","dontAsk","--no-session-persistence","--model",&model,
                        "--system-prompt","Read the requested local file with your tool and return its marker.",prompt]);
                }
                "CODEX" => {
                    command.env("CODEX_HOME",root.join("codex"));
                    command.args(["exec","--json","--skip-git-repo-check","--color","never","--sandbox","read-only",
                        "-c","approval_policy=\"never\"","-c","features.multi_agent=false","--model",&model,prompt]);
                }
                _ => {
                    command.env("OPENCODE_CONFIG",&opencode_config).env("OPENCODE_CONFIG_DIR",root.join("config"))
                        .env("AGENT_ROUTER_API_KEY",&token);
                    for key in ["OPENCODE_DISABLE_AUTOUPDATE","OPENCODE_DISABLE_MODELS_FETCH","OPENCODE_DISABLE_DEFAULT_PLUGINS",
                        "OPENCODE_DISABLE_LSP_DOWNLOAD","OPENCODE_DISABLE_CLAUDE_CODE","OPENCODE_DISABLE_AUTOCOMPACT","OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER"] { command.env(key,"true"); }
                    command.args(["--pure","run","--model",&format!("agent-router/{model}"),"--title","Agent Router live acceptance","--format","json",prompt]);
                }
            }
            let started = std::time::Instant::now();
            let result = tokio::time::timeout(Duration::from_secs(60), command.output()).await;
            let mut requests = vec![];
            while let Ok(event) = captured.try_recv() { requests.push(event); }
            let routed = requests.iter().all(|event| event.provider == provider && event.account_id == provider.as_str());
            let final_ok = requests.iter().any(|event| event.status == 200 && event.usage.output_tokens > 0);
            let replayed = requests.iter().any(|event| event.shape.as_ref().is_some_and(|shape| shape.blocks.iter().any(|block| block.kind == "tool_result" && block.text.contains(&marker))));
            if provider == Provider::OpenAI && !final_ok {
                for event in &requests {
                    println!("OpenAI capture: response_bytes={}, decoded_bytes={}, model={:?}, tools={:?}, error={:?}",
                        event.response_bytes,event.response_body.stored.len(),event.model,event.tool_calls,event.error);
                    let raw = String::from_utf8_lossy(&event.response_body.stored);
                    let frames = raw.lines().filter_map(|line| line.strip_prefix("data:").and_then(|data| serde_json::from_str::<Value>(data).ok()));
                    let terminal = frames.filter(|frame| frame["type"].as_str().is_some_and(|kind| kind.contains("completed") || kind.contains("done")))
                        .map(|frame| json!({"type":frame["type"],"status":frame["response"]["status"],
                            "input_tokens":frame["response"]["usage"]["input_tokens"],"output_tokens":frame["response"]["usage"]["output_tokens"]})).collect::<Vec<_>>();
                    println!("OpenAI captured terminal usage: {terminal:?}");
                }
            }
            let cli_ok = match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let ok = output.status.success() && stdout.contains(&marker);
                    if !ok { println!("{client}/{} CLI failure: {}", provider.as_str(), live_safe(&format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr)), &secrets)); }
                    ok
                }
                Ok(Err(error)) => { println!("{client}/{} could not run: {error}",provider.as_str()); false }
                Err(_) => { println!("{client}/{} timed out",provider.as_str()); false }
            };
            println!("{client}/{}: {}ms, statuses {:?}, routed={routed}, usage={final_ok}, tool_replay={replayed}, cli={cli_ok}",
                provider.as_str(),started.elapsed().as_millis(),requests.iter().map(|event|event.status).collect::<Vec<_>>());
            if !(cli_ok && routed && final_ok && replayed) { failures.push(format!("{client}/{} live tool cycle failed",provider.as_str())); }
            if requests.iter().any(|event| event.status == 429 || (event.provider == Provider::Kimi && event.status == 403
                && event.error.as_deref().and_then(crate::quota::kimi_spent_window).is_some())) {
                failures.push(format!("{} remaining clients deferred after provider rate limit",provider.as_str()));
                break;
            }
        }
    }
    native_client_revoke(&app, &token_id, &token, "claude-sonnet-4-6", "messages").await;
    server.abort();
    assert!(failures.is_empty(), "Live acceptance incomplete: {}",failures.join("; "));
}

// Real managed Codex and real machine tunnel; providers are isolated HTTP fixtures.
struct ManagedCodexCleanup {root:std::path::PathBuf}
struct ManagedPtyCleanup(Option<Box<dyn portable_pty::Child+Send+Sync>>);
impl Drop for ManagedPtyCleanup {
    fn drop(&mut self) {if let Some(child)=&mut self.0 {let _=child.kill();let _=child.wait();}}
}
impl Drop for ManagedCodexCleanup {
    fn drop(&mut self) {
        use std::io::{Read,Write};
        let cleanup=(||->anyhow::Result<()> {
            let control:Value=serde_json::from_slice(&std::fs::read(self.root.join("control.json"))?)?;
            let address:std::net::SocketAddr=control["address"].as_str().unwrap().parse()?;
            if address.ip()!=std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {return Ok(());}
            let wait=std::time::Duration::from_secs(2);
            let mut socket=std::net::TcpStream::connect_timeout(&address,wait)?;
            socket.set_read_timeout(Some(wait))?;socket.set_write_timeout(Some(wait))?;
            let mut bytes=serde_json::to_vec(&json!({"token":control["token"],"action":"stop"}))?;bytes.push(b'\n');socket.write_all(&bytes)?;
            let mut reply=[0u8;128];let _=socket.read(&mut reply)?;Ok(())
        })();
        let _=cleanup;
    }
}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
#[ignore="requires pinned native Codex and built router; check-primary-codex.sh runtime"]
async fn managed_codex_controls_cross_the_machine_tunnel() {
    use std::time::Duration;
    use axum::routing::post;
    use crate::codex::runtime::{Options,Host};
    let cli=std::env::var_os("AR_CODEX_BIN").expect("run the runtime gate");
    let router_binary=std::path::PathBuf::from(std::env::var_os("AR_ROUTER_BIN").expect("build the router before the runtime gate"));
    let lab=Lab::local("codex-runtime");
    let scratch=lab.state.config.db_path.parent().unwrap().join("managed");
    for dir in ["home","codex","config","state","cache","data","tmp","project","agent"] {std::fs::create_dir_all(scratch.join(dir)).unwrap();}
    let project=scratch.join("project").canonicalize().unwrap();
    std::fs::write(project.join("fixture.txt"),"LOCAL_TOOL_FIXTURE_OK\n").unwrap();
    let (observed,mut requests)=tokio::sync::mpsc::channel(128);
    let handler=move |req:Request<Body>| {let observed=observed.clone();async move {
        let (parts,body)=req.into_parts();
        let body:Value=serde_json::from_slice(&axum::body::to_bytes(body,MAX_REQUEST_BODY).await.unwrap()).unwrap();
        let output=codex_mock_stream(&body,parts.uri.path()=="/v1/responses");
        observed.send((parts.uri.path().to_owned(),parts.headers,body)).await.unwrap();
        Response::builder().header("content-type","text/event-stream").body(Body::from(output)).unwrap()
    }};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream=format!("http://{}",listener.local_addr().unwrap());
    let fake=axum::Router::new().route("/v1/messages",post(handler.clone())).route("/v1/responses",post(handler)).fallback(||async {StatusCode::FORBIDDEN});
    let fake_server=tokio::spawn(async move {axum::serve(listener,fake).await.unwrap()});
    let config=Config {server:true,bind:([127,0,0,1],0).into(),upstream:upstream.clone(),db_path:lab.state.config.db_path.clone(),body_cap:65536,dump_headers:0,allow_hosts:vec![],public_url:None,push_keys_dir:None,agents_dir:None,drain:Duration::from_secs(1),stop_delay:Duration::ZERO};
    let (capture,mut captured)=tokio::sync::mpsc::channel(128);
    let (network,_)=crate::network::switch();
    let mut state=AppState::new(config,capture,network).unwrap();
    let tls=hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
    Arc::get_mut(&mut state).unwrap().client=Client::builder(TokioExecutor::new()).build(tls);
    state.auth.pin(&state.config.db_path,NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
    let conn=rusqlite::Connection::open(&state.config.db_path).unwrap();
    crate::pricing::ensure_table(&conn).unwrap();
    let cases=[("claude-sonnet-4-6",Provider::Anthropic),("gpt-5.4",Provider::OpenAI),("kimi-for-coding",Provider::Kimi)];
    for (model,provider) in cases {
        let id=provider.as_str();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?1,1,1)",[id]).unwrap();
        conn.execute("INSERT INTO model_rate(model,key) VALUES(?1,NULL)",[model]).unwrap();
        state.credentials.write().await.insert(id.into(),Active {account_id:id.into(),label:id.into(),identity:None,token:format!("TEST-{id}-UPSTREAM"),expires_at_ms:0,provider,base_url:Some(upstream.clone()),openai_chatgpt:false,chatgpt_account_id:None});
    }
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router=format!("http://{}",listener.local_addr().unwrap());
    let app=crate::app(state.clone());let served=app.clone();
    let router_server=tokio::spawn(async move {axum::serve(listener,served.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap()});
    let (_,client_token)=native_client_token(&app,&state.config.db_path,"Managed Codex fixture").await;
    let response=native_client_request(&app,"POST","/api/auth/tokens",Some(NATIVE_SMOKE_ADMIN_PASSWORD),json!({"name":"Managed Codex machine","scope":"machine","expires_at":crate::db::now_ms()+1_200_000})).await;
    assert_eq!(response.status(),StatusCode::CREATED);
    let created:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
    let machine_token=created["token"].as_str().unwrap();
    let home=scratch.join("codex").canonicalize().unwrap();
    crate::codex::apply(&home.join("config.toml"),&scratch.join("kept.json"),&router,&client_token,true).unwrap();
    crate::codex::catalogue::sync(&home.join("config.toml"),&crate::models::client_catalogue(&conn).unwrap(),true).unwrap();
    // Trust only this disposable workspace in its isolated CODEX_HOME. Driving
    // the folder-trust screen through a PTY races native startup under CI load;
    // command approvals and questions are exercised explicitly below.
    let trust=format!("\n[projects.{}]\ntrust_level = \"trusted\"\n",serde_json::to_string(project.to_str().unwrap()).unwrap());
    std::fs::OpenOptions::new().append(true).open(home.join("config.toml")).unwrap().write_all(trust.as_bytes()).unwrap();
    std::fs::write(home.join("auth.json"),r#"{"OPENAI_API_KEY":"fixture-never-send-upstream"}"#).unwrap();
    let options=Options {root:home.join("agent-router-runtime"),backend:std::path::PathBuf::from(cli),codex_home:home.clone(),router_binary:router_binary.clone()};
    options.prepare().await.unwrap();
    let _cleanup=ManagedCodexCleanup {root:options.root.clone()};
    std::fs::write(scratch.join("agent/agent.json"),json!({"server":router,"password":machine_token,"machine":"managed-codex-fixture","label":"Managed Codex fixture","library":false,"update":false}).to_string()).unwrap();
    let mut paths=vec![options.backend.parent().unwrap().to_path_buf()];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let native_path=std::env::join_paths(paths).unwrap();
    let mut agent=tokio::process::Command::new(&router_binary).args(["agent","--data-dir"]).arg(scratch.join("agent"))
        .env_clear().env("PATH",&native_path).env("HOME",scratch.join("home")).env("CODEX_HOME",&home).env("TMPDIR",scratch.join("tmp"))
        .env("AGENT_ROUTER_PASSWORD",machine_token).env("CI","true").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(scratch.join("agent.log")).unwrap()).kill_on_drop(true).spawn().unwrap();
    async fn api(app:&axum::Router,path:&str,body:Value)->(StatusCode,Value) {
        let method=if body.is_null() {"GET"} else {"POST"};
        let response=native_client_request(app,method,path,Some(NATIVE_SMOKE_ADMIN_PASSWORD),body).await;
        let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();
        (status,serde_json::from_slice(&bytes).unwrap_or_else(|_|panic!("{path}: {status}: {}",String::from_utf8_lossy(&bytes))))
    }
    async fn ok(app:&axum::Router,path:&str,body:Value)->Value {let (status,value)=api(app,path,body).await;assert!(status.is_success(),"{path}: {status}: {value}");value}
    let ready_by=tokio::time::Instant::now()+Duration::from_secs(60);
    while tokio::time::Instant::now()<ready_by {
        if !state.machines.connected().is_empty() {break;}
        if let Some(status)=agent.try_wait().unwrap() {
            panic!("fixture agent exited {status}: {}",std::fs::read_to_string(scratch.join("agent.log")).unwrap());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!state.machines.connected().is_empty(),"machine tunnel: {}",std::fs::read_to_string(scratch.join("agent.log")).unwrap());
    let mut sessions=Vec::new();
    for (model,provider) in cases {
        let made=ok(&app,"/api/live/start",json!({"client":"codex","machine":"managed-codex-fixture","cwd":project,"model":model})).await;
        let id=made["session"].as_str().unwrap().to_owned();assert!(id.starts_with("codex:"));
        let listed=ok(&app,"/api/live/sessions",Value::Null).await;
        assert!(listed.as_array().unwrap().iter().any(|r|r["session_id"]==id && r["hosted"]==true && r["wrapped"]==true),"new managed thread must be listed and controllable: {listed}");
        let sent=ok(&app,"/api/live/send",json!({"session":id,"text":"Read fixture.txt using a command tool and report its content."})).await;
        assert_eq!(sent["acknowledged"],true);
        for turn in 0..2 {
            let (path,headers,body)=tokio::time::timeout(Duration::from_secs(30),requests.recv()).await.unwrap().unwrap();
            assert_eq!(path,if provider==Provider::OpenAI {"/v1/responses"} else {"/v1/messages"});assert_eq!(body["model"],model);
            assert_eq!(header_session_of(&headers).as_deref(),Some(id.as_str()));
            if turn==1 {assert!(body.to_string().contains("LOCAL_TOOL_FIXTURE_OK"),"native command must execute in the thread workspace: {body}");}
            for secret in [&client_token,machine_token,NATIVE_SMOKE_ADMIN_PASSWORD] {assert!(!body.to_string().contains(secret));assert!(headers.values().all(|v|v.to_str().is_ok_and(|s|!s.contains(secret))));}
            let event=tokio::time::timeout(Duration::from_secs(10),captured.recv()).await.unwrap().unwrap();
            assert_eq!(event.session_id.as_deref(),Some(id.as_str()));assert_eq!(event.provider,provider);assert_eq!(event.status,200);assert_eq!(event.usage.input_tokens,42);assert_eq!(event.usage.output_tokens,13);
        }
        sessions.push(id);println!("MANAGED_CODEX_PROVIDER_OK {model}");
    }
    let id=&sessions[1];
    for _ in 0..100 {let rows=ok(&app,"/api/live/sessions",Value::Null).await;if rows.as_array().unwrap().iter().any(|r|r["session_id"]==id.as_str() && r["active"]==false) {break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    ok(&app,"/api/live/model",json!({"session":id,"model":"kimi-for-coding"})).await;
    ok(&app,"/api/live/effort",json!({"session":id,"effort":"low"})).await;
    ok(&app,"/api/live/mode",json!({"session":id,"mode":"default"})).await;
    ok(&app,"/api/live/send",json!({"session":id,"text":"CONTROL_APPROVAL: write the approval proof using a command."})).await;
    let mut approval=Value::Null;
    for _ in 0..100 {let value=ok(&app,&format!("/api/live/prompt?session={id}"),Value::Null).await;if value["pending"]==true {approval=value;break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    assert_eq!(approval["question"],false,"native approval must arrive: {approval}");
    let wrong=api(&app,"/api/live/answer",json!({"session":sessions[0],"request_id":approval["request_id"],"allow":true})).await;assert_eq!(wrong.0,StatusCode::CONFLICT);
    ok(&app,"/api/live/answer",json!({"session":id,"request_id":approval["request_id"],"allow":true})).await;
    for _ in 0..100 {
        if project.join("approval-proof.txt").exists() {break;}
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(std::fs::read_to_string(project.join("approval-proof.txt")).unwrap(),"CONTROL_APPROVED");
    assert_eq!(api(&app,"/api/live/answer",json!({"session":id,"request_id":approval["request_id"],"allow":true})).await.0,StatusCode::CONFLICT);
    // Complete the approved turn before changing mode, retaining exactly the same thread.
    for _ in 0..100 {let rows=ok(&app,"/api/live/sessions",Value::Null).await;if rows.as_array().unwrap().iter().any(|r|r["session_id"]==id.as_str() && r["active"]==false) {break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    ok(&app,"/api/live/model",json!({"session":id,"model":"gpt-5.4"})).await;
    ok(&app,"/api/live/mode",json!({"session":id,"mode":"plan"})).await;
    ok(&app,"/api/live/send",json!({"session":id,"text":"CONTROL_QUESTION: ask which color to use."})).await;
    let mut question=Value::Null;
    for _ in 0..100 {let value=ok(&app,&format!("/api/live/prompt?session={id}"),Value::Null).await;if value["pending"]==true {question=value;break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    assert_eq!(question["question"],true,"native question: {question}");
    ok(&app,"/api/live/answer",json!({"session":id,"request_id":question["request_id"],"answers":{"Which color should we use?":"Blue"}})).await;
    for _ in 0..100 {let rows=ok(&app,"/api/live/sessions",Value::Null).await;if rows.as_array().unwrap().iter().any(|r|r["session_id"]==id.as_str() && r["active"]==false) {break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    ok(&app,"/api/live/mode",json!({"session":id,"mode":"acceptEdits"})).await;
    ok(&app,"/api/live/send",json!({"session":id,"text":"CONTROL_SLEEP: wait using a command."})).await;
    ok(&app,"/api/live/send",json!({"session":id,"text":"Steer the current turn to stop waiting."})).await;
    ok(&app,"/api/live/interrupt",json!({"session":id})).await;
    println!("MANAGED_CODEX_CONTROLS_OK approvals=1 questions=1 steer=1 cancel=1");
    // Private supervisor control cannot be used with a guessed local token.
    let control:Value=serde_json::from_slice(&std::fs::read(options.root.join("control.json")).unwrap()).unwrap();
    use tokio::io::AsyncWriteExt;
    let mut probe=tokio::net::TcpStream::connect(control["address"].as_str().unwrap()).await.unwrap();
    probe.write_all(b"{\"token\":\"wrong\",\"action\":\"stop\"}\n").await.unwrap();drop(probe);
    assert!(ok(&app,"/api/live/sessions",Value::Null).await.as_array().unwrap().iter().any(|r|r["session_id"]==id.as_str() && r["wrapped"]==true));
    crate::codex::runtime::stop(&options).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(api(&app,"/api/live/send",json!({"session":id,"text":"Must not start a new backend implicitly."})).await.0,StatusCode::SERVICE_UNAVAILABLE);
    ok(&app,"/api/live/resume",json!({"session":id})).await;
    let stats=ok(&app,&format!("/api/live/stats?session={id}"),Value::Null).await;assert!(stats["turns"].as_i64().unwrap()>0);
    ok(&app,"/api/live/archive",json!({"session":id,"archived":true})).await;
    ok(&app,"/api/live/archive",json!({"session":id,"archived":false})).await;
    ok(&app,"/api/live/resume",json!({"session":id})).await;
    assert_eq!(native_client_request(&app,"POST","/api/live/send",Some(&client_token),json!({"session":id,"text":"unauthorized"})).await.status(),StatusCode::FORBIDDEN);
    assert_eq!(api(&app,"/api/live/send",json!({"session":"codex:unmanaged","text":"must be refused"})).await.0,StatusCode::NOT_FOUND);
    println!("MANAGED_CODEX_RECOVERY_OK same-thread=1 offline-refusal=1 archive=1 scopes=1");
    // The actual native terminal UI attaches through --remote, rather than a
    // protocol stand-in pretending to be a CLI. It shares this backend and thread
    // identity with the machine controller after its terminal closes.
    use std::io::{Read,Write};
    let pair=portable_pty::native_pty_system().openpty(portable_pty::PtySize {rows:36,cols:120,pixel_width:0,pixel_height:0}).unwrap();
    let mut command=portable_pty::CommandBuilder::new(&options.backend);
    command.args(["--remote",&format!("unix://{}",options.socket().display()),"--model","gpt-5.4","--no-alt-screen","--ask-for-approval","never","--sandbox","workspace-write","--cd",project.to_str().unwrap(),"MANAGED_CLI_PROMPT: Read fixture.txt with a command tool."]);
    command.env_clear();command.env("PATH",&native_path);command.env("HOME",scratch.join("home"));command.env("CODEX_HOME",&home);command.env("TMPDIR",scratch.join("tmp"));command.env("TERM","xterm-256color");command.env("LANG","en_US.UTF-8");
    let child=pair.slave.spawn_command(command).unwrap();let mut tty=ManagedPtyCleanup(Some(child));drop(pair.slave);
    let mut tty_reader=pair.master.try_clone_reader().unwrap();
    let tty_writer=Arc::new(std::sync::Mutex::new(pair.master.take_writer().unwrap()));
    let tty_output=Arc::new(std::sync::Mutex::new(String::new()));
    let collector={let output=tty_output.clone();let writer=tty_writer.clone();std::thread::spawn(move || {
        let mut bytes=[0u8;4096];
        while let Ok(n)=tty_reader.read(&mut bytes) {
            if n==0 {break;}
            let text=String::from_utf8_lossy(&bytes[..n]);
            if text.contains("\u{1b}[6n") {let _=writer.lock().unwrap().write_all(b"\x1b[1;1R");}
            if text.contains("\u{1b}[c") {let _=writer.lock().unwrap().write_all(b"\x1b[?1;2c");}
            let mut log=output.lock().unwrap();if log.len()<1<<20 {log.push_str(&text);}
        }
    })};
    let mut cli_rows=Vec::new();let mut other_requests=Vec::new();
    let cli_ready_by=tokio::time::Instant::now()+Duration::from_secs(60);
    while tokio::time::Instant::now()<cli_ready_by {
        while let Ok(row)=requests.try_recv() {
            let meta:Value=row.1.get("x-codex-turn-metadata").and_then(|v|v.to_str().ok()).and_then(|s|serde_json::from_str(s).ok()).unwrap_or(Value::Null);
            if row.2.to_string().contains("MANAGED_CLI_PROMPT") && meta["thread_source"]!="thread_title" {cli_rows.push(row);} else {other_requests.push(row);}
        }
        if cli_rows.len()>=2 {break;}
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(cli_rows.len(),2,"native remote CLI must execute a tool turn: {}",tty_output.lock().unwrap());
    let cli_session=header_session_of(&cli_rows[0].1).unwrap();
    assert_eq!(header_session_of(&cli_rows[1].1).as_deref(),Some(cli_session.as_str()));
    assert!(cli_rows[1].2.to_string().contains("LOCAL_TOOL_FIXTURE_OK"));
    for _ in 0..100 {
        if tty_output.lock().unwrap().contains("SMOKE_OK") {break;}
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(tty_output.lock().unwrap().contains("SMOKE_OK"),"native terminal must display the result");
    tty_writer.lock().unwrap().write_all(b"\x03").unwrap();tokio::time::sleep(Duration::from_millis(200)).await;let _=tty_writer.lock().unwrap().write_all(b"\x03");
    for _ in 0..50 {if tty.0.as_mut().unwrap().try_wait().unwrap().is_some() {break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    let _=tty.0.as_mut().unwrap().kill();let _=tty.0.as_mut().unwrap().wait();tty.0=None;drop(pair.master);drop(tty_writer);collector.join().unwrap();
    let rows=ok(&app,"/api/live/sessions",Value::Null).await;
    assert!(rows.as_array().unwrap().iter().any(|r|r["session_id"]==cli_session && r["hosted"]==true));
    ok(&app,"/api/live/send",json!({"session":cli_session,"text":"REMOTE_CLI_FOLLOWUP: Read fixture.txt again."})).await;
    println!("MANAGED_NATIVE_CLI_OK same-thread=1 tool-replay=1 remotely-controlled=1");
    if let Some(driver)=std::env::var_os("AR_CODEX_MANAGED_DRIVER") {
        let output=tokio::time::timeout(Duration::from_secs(300),tokio::process::Command::new("node").arg(driver)
            .env("AR_SIDEBAR_SCRATCH",&scratch).env("AR_SIDEBAR_ROUTER",&router).env("AR_SIDEBAR_TOKEN",&client_token)
            .env("AR_SIDEBAR_ADMIN",NATIVE_SMOKE_ADMIN_PASSWORD).env("AR_SIDEBAR_MODELS",serde_json::to_string(&cases.iter().map(|(m,p)|json!({"model":m,"provider":p.as_str()})).collect::<Vec<_>>()).unwrap())
            .env("AR_ROUTER_BIN",&router_binary).env("AR_CODEX_BIN",&options.backend).env("AR_MANAGED_SOCKET",options.socket())
            .kill_on_drop(true).output()).await.expect("managed native sidebar timed out").unwrap();
        assert!(output.status.success(),"{}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        let stdout=String::from_utf8_lossy(&output.stdout);println!("{stdout}");assert!(stdout.contains("MANAGED_SIDEBAR_OK"));
    }
    // Every extra fixture stream remains scoped to a managed conversation, including
    // the approval, question, cancelled turn and any native frontend title traffic.
    while let Ok(row)=requests.try_recv() {other_requests.push(row);}
    if std::env::var_os("AR_CODEX_MANAGED_DRIVER").is_some() {
        let artifacts=std::path::PathBuf::from(std::env::var_os("AR_SIDEBAR_ARTIFACTS").unwrap());
        let sessions:Value=serde_json::from_slice(&std::fs::read(artifacts.join("managed-sidebar-sessions.json")).unwrap()).unwrap();
        std::fs::write(artifacts.join("managed-routed-requests.json"),serde_json::to_vec_pretty(&other_requests.iter().map(|(path,headers,body)|json!({"path":path,"session":header_session_of(headers),"body":body})).collect::<Vec<_>>()).unwrap()).unwrap();
        for session in sessions.as_array().unwrap() {
            let id=session["session"].as_str().unwrap();let model=session["model"].as_str().unwrap();
            let rows=other_requests.iter().filter(|(_,headers,body)|header_session_of(headers).as_deref()==Some(id) && body["model"]==model).collect::<Vec<_>>();
            assert_eq!(rows.len(),4,"{model}: native sidebar and remote followup each need a prompt/tool replay pair");
            assert!(rows[0].2.to_string().contains("MANAGED_SELECTION_CONTEXT_789"),"{model}: actual editor selection must reach the provider");
            for index in [1,3] {assert!(rows[index].2.to_string().contains("LOCAL_TOOL_FIXTURE_OK"));}
        }
        println!("MANAGED_SIDEBAR_PACKETS_OK editor-context=3 tool-replay=6 same-thread=3");
    }
    for (_,headers,body) in other_requests {
        assert!(header_session_of(&headers).unwrap().starts_with("codex:"));
        assert!(!body.to_string().contains(&client_token));assert!(!body.to_string().contains(machine_token));
    }
    while let Ok(event)=captured.try_recv() {assert!(event.session_id.unwrap().starts_with("codex:"));assert_eq!(event.status,200);}
    let local=Host::new(options.clone());if local.connect(false).await.is_ok() {crate::codex::runtime::stop(&options).await.unwrap();}
    agent.kill().await.unwrap();agent.wait().await.unwrap();router_server.abort();fake_server.abort();
    println!("MANAGED_CODEX_RUNTIME_OK");
}

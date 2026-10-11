//! Owned Codex app-server, reached over its native local stdio/socket proxy.
//! Only threads created or explicitly resumed on this backend take controls.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Notify};
use futures_util::{SinkExt,StreamExt};
use tokio_tungstenite::tungstenite::Message;

struct NativePipe {input:ChildStdin,output:tokio::process::ChildStdout}
impl tokio::io::AsyncRead for NativePipe {
    fn poll_read(self:std::pin::Pin<&mut Self>,cx:&mut std::task::Context<'_>,buf:&mut tokio::io::ReadBuf<'_>)->std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().output).poll_read(cx,buf)
    }
}
impl tokio::io::AsyncWrite for NativePipe {
    fn poll_write(self:std::pin::Pin<&mut Self>,cx:&mut std::task::Context<'_>,buf:&[u8])->std::task::Poll<std::io::Result<usize>> {std::pin::Pin::new(&mut self.get_mut().input).poll_write(cx,buf)}
    fn poll_flush(self:std::pin::Pin<&mut Self>,cx:&mut std::task::Context<'_>)->std::task::Poll<std::io::Result<()>> {std::pin::Pin::new(&mut self.get_mut().input).poll_flush(cx)}
    fn poll_shutdown(self:std::pin::Pin<&mut Self>,cx:&mut std::task::Context<'_>)->std::task::Poll<std::io::Result<()>> {std::pin::Pin::new(&mut self.get_mut().input).poll_shutdown(cx)}
}
type Socket=tokio_tungstenite::WebSocketStream<NativePipe>;
enum Writer {
    Socket(futures_util::stream::SplitSink<Socket,Message>),
    #[cfg(test)] Stdio(ChildStdin),
}
enum Reader {
    Socket(futures_util::stream::SplitStream<Socket>),
    #[cfg(test)] Stdio(BufReader<tokio::process::ChildStdout>),
}

async fn socket(child: &mut Child) -> Result<Socket> {
    let pipe=NativePipe {input:child.stdin.take().unwrap(),output:child.stdout.take().unwrap()};
    let config=tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_LINE)).max_frame_size(Some(MAX_LINE));
    Ok(tokio::time::timeout(WAIT,tokio_tungstenite::client_async_with_config("ws://localhost/",pipe,Some(config))).await.context("Codex's local socket did not accept its WebSocket handshake")??.0)
}

impl Reader {
    async fn next(&mut self) -> Result<Option<Value>> {
        loop {
            match self {
                Self::Socket(stream)=>match stream.next().await {
                    Some(Ok(Message::Text(text)))=>return Ok(Some(serde_json::from_str(&text)?)),
                    Some(Ok(Message::Close(_)))|None=>return Ok(None),
                    Some(Ok(Message::Ping(_)|Message::Pong(_)))=>continue,
                    Some(Ok(_))=>bail!("unexpected Codex socket frame"),
                    Some(Err(error))=>return Err(error.into()),
                },
                #[cfg(test)] Self::Stdio(reader)=> {
                    let mut line=Vec::new();
                    let n=tokio::io::AsyncReadExt::take(reader,(MAX_LINE+1) as u64).read_until(b'\n',&mut line).await?;
                    if n==0 {return Ok(None);}
                    if n>MAX_LINE || line.last()!=Some(&b'\n') {bail!("invalid stdio frame");}
                    return Ok(Some(serde_json::from_slice(&line)?));
                }
            }
        }
    }
}

const WAIT: Duration = Duration::from_secs(10);
const MAX_LINE: usize = 8 * 1024 * 1024;
#[cfg(test)]
static CONNECTION: AtomicU64 = AtomicU64::new(1);
type Reply = oneshot::Sender<std::result::Result<Value, String>>;

#[derive(Clone)]
pub struct Pending {
    pub key: String,
    pub id: Value,
    pub method: String,
    pub params: Value,
    resolving: bool,
}

#[derive(Default)]
struct Observed {
    threads: BTreeMap<String, Value>,
    turns: HashMap<String, String>,
    pending: BTreeMap<String, Pending>,
    subscribed: HashSet<String>,
}

pub struct Rpc {
    writer: tokio::sync::Mutex<Writer>,
    _child: tokio::sync::Mutex<Child>,
    replies: Mutex<HashMap<String, Reply>>,
    observed: Mutex<Observed>,
    pub changed: Notify,
    alive: AtomicBool,
    sequence: AtomicU64,
    epoch: String,
    state_path: PathBuf,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn private_file(path: &Path, value: &[u8]) -> Result<()> {
    private_file_mode(path,value,false)
}

fn private_file_mode(path: &Path, value: &[u8], executable: bool) -> Result<()> {
    #[cfg(windows)] let _=executable;
    use std::io::Write;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() { bail!("refusing an unmanaged runtime file"); }
    }
    let temporary=path.with_extension(format!("{}.tmp",std::process::id()));
    let mut options=std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(if executable {0o700} else {0o600}); }
    let mut file=options.open(&temporary)?;
    let result=(|| { file.write_all(value)?; file.sync_all()?; std::fs::rename(&temporary,path) })();
    if result.is_err() { let _=std::fs::remove_file(&temporary); }
    result?;
    Ok(())
}

fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?; }
        #[cfg(windows)] std::fs::create_dir_all(path)?;
    }
    let meta=std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() { bail!("runtime directory must be an ordinary private directory"); }
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if meta.uid()!=unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 { bail!("runtime directory must be owned by this user with mode 0700"); }
    }
    Ok(())
}

impl Rpc {
    async fn connect(options: &Options) -> Result<Arc<Self>> {
        Self::open(options,false).await
    }
    async fn open(options: &Options, _test_stdio: bool) -> Result<Arc<Self>> {
        let mut child=options.command().args(["app-server","proxy","--sock"])
            .arg(options.socket()).stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .kill_on_drop(true).spawn().context("connecting Codex's native stdio proxy")?;
        #[cfg(test)]
        let (writer,mut reader)=if _test_stdio {
            (Writer::Stdio(child.stdin.take().unwrap()),Reader::Stdio(BufReader::new(child.stdout.take().unwrap())))
        } else {let (write,read)=socket(&mut child).await?.split();(Writer::Socket(write),Reader::Socket(read))};
        #[cfg(not(test))]
        let (writer,mut reader)={let (write,read)=socket(&mut child).await?.split();(Writer::Socket(write),Reader::Socket(read))};
        let state_path=options.root.join("threads.json");
        let mut observed=Observed::default();
        if let Ok(raw)=std::fs::read(&state_path) { observed.threads=serde_json::from_slice(&raw)?; }
        for thread in observed.threads.values_mut() {thread["status"]=json!({"type":"notLoaded"});}
        use ring::rand::SecureRandom;
        let mut random=[0u8;16];ring::rand::SystemRandom::new().fill(&mut random).map_err(|_|anyhow!("system randomness unavailable"))?;
        let epoch=random.iter().map(|b|format!("{b:02x}")).collect::<String>();
        let rpc=Arc::new(Self {
            writer:tokio::sync::Mutex::new(writer), _child:tokio::sync::Mutex::new(child),
            replies:Mutex::new(HashMap::new()), observed:Mutex::new(observed), changed:Notify::new(),
            alive:AtomicBool::new(true), sequence:AtomicU64::new(1),
            epoch,state_path,
        });
        let weak=Arc::downgrade(&rpc);
        tokio::spawn(async move {
            loop {
                let read=reader.next().await;
                let Some(rpc)=weak.upgrade() else { break };
                match read {
                    Ok(Some(message))=>rpc.receive(message),
                    _=>{rpc.disconnect();break;}
                }
            }
        });
        let initialized=rpc.call("initialize",json!({"clientInfo":{"name":"agent_router_codex","title":"Agent Router","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        let version=initialized["userAgent"].as_str().unwrap_or("");
        if !version.contains(super::catalogue::QUALIFIED_VERSION) { bail!("the managed app-server is not the qualified Codex version"); }
        rpc.write(json!({"method":"initialized"})).await?;
        Ok(rpc)
    }

    pub fn alive(&self) -> bool { self.alive.load(Ordering::Acquire) }

    fn disconnect(&self) {
        self.alive.store(false,Ordering::Release);
        for (_,reply) in locked(&self.replies).drain() { let _=reply.send(Err("Codex disconnected; the action was not acknowledged".into())); }
        locked(&self.observed).pending.clear();
        locked(&self.observed).turns.clear();
        self.changed.notify_waiters();
    }

    fn receive(&self, message: Value) {
        if let Some(id)=message.get("id") {
            if message.get("method").is_none() {
                if let Some(reply)=locked(&self.replies).remove(&id.to_string()) {
                    let answer=match (message.get("error"),message.get("result")) {
                        (Some(error),None)=>Err(error["message"].as_str().unwrap_or("Codex refused this action").to_owned()),
                        (None,Some(result))=>Ok(result.clone()),
                        _=>Err("invalid native reply: no unambiguous acknowledgement".into()),
                    };
                    let _=reply.send(answer);
                }
            } else {
                let method=message["method"].as_str().unwrap_or("").to_owned();
                let params=message["params"].clone();
                // A request is actionable only when Codex sent it on this connection
                // for a managed thread. Other ceremonies stay with their native owner.
                let mut seen=locked(&self.observed);
                if params["threadId"].as_str().is_some_and(|id|seen.threads.contains_key(id)) {
                    let key=format!("{}:{}",self.epoch,id);
                    seen.pending.insert(key.clone(),Pending {key,id:id.clone(),method,params,resolving:false});
                }
            }
        } else {
            let method=message["method"].as_str().unwrap_or("");
            let params=&message["params"];
            let mut seen=locked(&self.observed);
            match method {
                "thread/started"=> {
                    let mut thread=params["thread"].clone();
                    if thread["parentThreadId"].is_null() && thread["threadSource"]!="thread_title" {
                        if let Some(id)=thread["id"].as_str().map(str::to_owned) {
                            if let Some(object)=thread.as_object_mut() { object.remove("turns"); }
                            seen.threads.insert(id,thread);
                            let _=private_file(&self.state_path,&serde_json::to_vec(&seen.threads).unwrap());
                        }
                    }
                }
                "turn/started"=> { if let (Some(id),Some(turn))=(params["threadId"].as_str(),params["turn"]["id"].as_str()) { seen.turns.insert(id.into(),turn.into()); } }
                "turn/completed"=> { if let Some(id)=params["threadId"].as_str() { seen.turns.remove(id);seen.pending.retain(|_,p|p.params["threadId"]!=id); } }
                "thread/status/changed"=> {if let Some(thread)=params["threadId"].as_str().and_then(|id|seen.threads.get_mut(id)) {thread["status"]=params["status"].clone();}}
                "thread/settings/updated"=> {if let Some(thread)=params["threadId"].as_str().and_then(|id|seen.threads.get_mut(id)) {
                    thread["model"]=params["threadSettings"]["model"].clone();thread["reasoningEffort"]=params["threadSettings"]["effort"].clone();
                }}
                "serverRequest/resolved"=> { let key=format!("{}:{}",self.epoch,params["requestId"]);seen.pending.remove(&key); }
                _=>{}
            }
        }
        self.changed.notify_waiters();
    }

    async fn write(&self, message: Value) -> Result<()> {
        if !self.alive() { bail!("Codex is disconnected"); }
        let text=serde_json::to_string(&message)?;
        if text.len()>MAX_LINE { bail!("Codex request is too large"); }
        let sent=tokio::time::timeout(WAIT,async {
            match &mut *self.writer.lock().await {
                Writer::Socket(writer)=>writer.send(Message::Text(text.into())).await.map_err(anyhow::Error::from),
                #[cfg(test)] Writer::Stdio(writer)=>writer.write_all(format!("{text}\n").as_bytes()).await.map_err(anyhow::Error::from),
            }
        }).await;
        if !matches!(sent,Ok(Ok(()))) { self.disconnect();bail!("Codex disconnected while writing; do not retry an unacknowledged action automatically"); }
        Ok(())
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id=json!(format!("ar-{}-{}",self.epoch,self.sequence.fetch_add(1,Ordering::Relaxed)));
        let (send,recv)=oneshot::channel();
        locked(&self.replies).insert(id.to_string(),send);
        if let Err(error)=self.write(json!({"id":id,"method":method,"params":params})).await { locked(&self.replies).remove(&id.to_string());return Err(error); }
        let answer=tokio::time::timeout(WAIT,recv).await;
        locked(&self.replies).remove(&id.to_string());
        match answer {
            Ok(Ok(Ok(value)))=>Ok(value),
            Ok(Ok(Err(reason)))=>Err(anyhow!(reason)),
            _=>bail!("Codex did not acknowledge this action; do not retry it automatically"),
        }
    }

    pub fn pending(&self, native: &str) -> Vec<Pending> {
        locked(&self.observed).pending.values().filter(|p|p.params["threadId"]==native && !p.resolving).cloned().collect()
    }

    pub async fn answer(&self, native: &str, key: &str, result: Value) -> Result<()> {
        let id={
            let mut observed=locked(&self.observed);
            let pending=observed.pending.get_mut(key).context("that Codex request is no longer waiting")?;
            if pending.params["threadId"]!=native || pending.resolving { bail!("that Codex request belongs to another thread or is already being answered"); }
            pending.resolving=true;
            pending.id.clone()
        };
        self.write(json!({"id":id,"result":result})).await?;
        tokio::time::timeout(WAIT,async {
            loop {
                let notified=self.changed.notified();
                if !self.alive() { bail!("Codex disconnected before resolving the approval"); }
                if !locked(&self.observed).pending.contains_key(key) { return Ok(()); }
                notified.await;
            }
        }).await.context("Codex did not acknowledge resolving the approval")?
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Options {
    pub root: PathBuf,
    pub backend: PathBuf,
    pub codex_home: PathBuf,
    pub router_binary: PathBuf,
}

impl Options {
    pub fn socket(&self) -> PathBuf { self.root.join("app-server.sock") }
    fn command(&self) -> Command {
        let mut command=Command::new(&self.backend);
        command.env("CODEX_HOME",&self.codex_home);
        for key in ["AGENT_ROUTER_SERVER_PASSWORD","AGENT_ROUTER_PASSWORD","AGENT_ROUTER_TOKEN","AGENT_ROUTER_CLIENT_TOKEN","NORTHSTAR_SESSION","CODEX_THREAD_ID"] { command.env_remove(key); }
        command
    }
    pub fn here() -> Result<Self> {
        let home=crate::accounts::codex_home()?.canonicalize()?;
        let root=home.join("agent-router-runtime");
        if let Ok(raw)=std::fs::read(root.join("options.json")) {
            let saved:Self=serde_json::from_slice(&raw)?;
            if saved.root!=root || saved.codex_home!=home { bail!("managed Codex runtime belongs to another home"); }
            return Ok(saved);
        }
        let backend=crate::clients::here().into_iter().find(|c|c.client==crate::clients::CODEX)
            .and_then(|c|c.cli).map(|cli|PathBuf::from(cli.path)).context("no native Codex CLI was found; configure the managed runtime first")?;
        Ok(Self {root,backend:backend.canonicalize()?,codex_home:home,router_binary:std::env::current_exe()?})
    }
    pub async fn prepare(&self) -> Result<()> {
        private_dir(&self.root)?;
        if self.backend==self.router_binary || self.backend.file_stem().is_some_and(|s|s=="agent-router-codex") {bail!("choose the real native Codex binary, not the Agent Router adapter");}
        let output=self.command().arg("--version").output().await?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim()!=format!("codex-cli {}",super::catalogue::QUALIFIED_VERSION) { bail!("managed control requires Codex {}",super::catalogue::QUALIFIED_VERSION); }
        private_file(&self.root.join("options.json"),&serde_json::to_vec(self)?)
    }
    pub fn launcher(&self) -> PathBuf {self.root.join(if cfg!(windows) {"agent-router-codex.exe"} else {"agent-router-codex"})}
    fn install_launcher(&self) -> Result<()> {
        let source=std::fs::read(&self.router_binary)?;
        if std::fs::read(self.launcher()).is_ok_and(|existing|existing==source) {return Ok(());}
        private_file_mode(&self.launcher(),&source,true)?;
        Ok(())
    }
}

pub struct Host {
    pub options: Options,
    connection: tokio::sync::Mutex<Option<Arc<Rpc>>>,
}

impl Host {
    pub fn new(options: Options) -> Self { Self {options,connection:tokio::sync::Mutex::new(None)} }
    pub async fn connect(&self, start: bool) -> Result<Arc<Rpc>> {
        let mut held=self.connection.lock().await;
        if let Some(rpc)=held.as_ref().filter(|r|r.alive()) { return Ok(rpc.clone()); }
        self.options.prepare().await?;
        if let Ok(rpc)=Rpc::connect(&self.options).await { *held=Some(rpc.clone());return Ok(rpc); }
        if !start { bail!("managed Codex backend is offline; reconnect or explicitly resume the session"); }
        Command::new(&self.options.router_binary).args(["codex","supervise"])
            .arg(self.options.root.join("options.json"))
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).spawn().context("starting the managed Codex supervisor")?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(20);
        while tokio::time::Instant::now()<deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let attempt=tokio::time::timeout_at(deadline,Rpc::connect(&self.options)).await;
            match attempt {
                Ok(Ok(rpc))=>{*held=Some(rpc.clone());return Ok(rpc);}
                Ok(Err(error))=>tracing::debug!(%error,"waiting for managed Codex startup"),
                Err(_)=>break,
            }
        }
        bail!("managed Codex did not start; inspect its private app-server.log")
    }
    pub fn hosts(&self, session: &str) -> bool {
        let Some(id)=super::session::native_id(session) else { return false };
        self.connection.try_lock().ok().and_then(|held|held.as_ref().map(|r|locked(&r.observed).threads.contains_key(id))).unwrap_or_else(|| {
            std::fs::read(self.options.root.join("threads.json")).ok().and_then(|raw|serde_json::from_slice::<BTreeMap<String,Value>>(&raw).ok()).is_some_and(|threads|threads.contains_key(id))
        })
    }
    pub async fn start(&self, cwd: &Path, model: &str) -> Result<String> {
        if !cwd.is_absolute() || !cwd.is_dir() { bail!("start Codex in an existing directory by its full path"); }
        let rpc=self.connect(true).await?;
        let result=rpc.call("thread/start",json!({"cwd":cwd,"model":model,"modelProvider":"agent-router","approvalPolicy":"on-request","sandbox":"workspace-write"})).await?;
        self.remember(&rpc,&result["thread"])?;
        Ok(super::session::router_id(result["thread"]["id"].as_str().context("Codex named no thread")?))
    }
    fn remember(&self, rpc: &Rpc, thread: &Value) -> Result<()> {
        let id=thread["id"].as_str().context("Codex named no thread")?;
        let mut thread=thread.clone();if let Some(object)=thread.as_object_mut() {object.remove("turns");}
        let mut seen=locked(&rpc.observed);seen.threads.insert(id.to_owned(),thread);seen.subscribed.insert(id.to_owned());
        private_file(&rpc.state_path,&serde_json::to_vec(&seen.threads)?)
    }
    pub async fn resume(&self, session: &str) -> Result<Value> {
        let id=super::session::native_id(session).context("invalid Codex thread id")?;
        let rpc=self.connect(true).await?;
        let result=rpc.call("thread/resume",json!({"threadId":id,"excludeTurns":true})).await?;
        self.remember(&rpc,&result["thread"])?;
        Ok(result)
    }
    pub async fn thread(&self, session: &str) -> Result<Value> {
        if !self.hosts(session) { bail!("{}",super::live::WATCHED); }
        let rpc=self.connect(false).await?;
        let id=super::session::native_id(session).unwrap();
        let result=rpc.call("thread/read",json!({"threadId":id,"includeTurns":false})).await?;
        if result["thread"]["status"]["type"]=="notLoaded" {bail!("this Codex thread is not loaded; explicitly resume it before sending controls");}
        if !locked(&rpc.observed).subscribed.contains(id) {
            let resumed=rpc.call("thread/resume",json!({"threadId":id,"excludeTurns":true})).await?;
            self.remember(&rpc,&resumed["thread"])?;
        }
        self.remember(&rpc,&result["thread"])?;
        Ok(result["thread"].clone())
    }
    pub async fn send(&self, session: &str, text: &str) -> Result<Value> {
        let thread=self.thread(session).await?;
        let rpc=self.connect(false).await?;
        let input=json!([{"type":"text","text":text,"text_elements":[]}]);
        if thread["status"]["type"]=="active" {
            let id=self.active_turn(&rpc,super::session::native_id(session).unwrap()).await?;
            rpc.call("turn/steer",json!({"threadId":thread["id"],"expectedTurnId":id,"input":input})).await
        } else {
            rpc.call("turn/start",json!({"threadId":thread["id"],"input":input,"turnTrigger":"agent-router"})).await
        }
    }
    pub async fn interrupt(&self, session: &str) -> Result<Value> {
        self.thread(session).await?;
        let rpc=self.connect(false).await?;
        let id=super::session::native_id(session).unwrap();
        let turn=self.active_turn(&rpc,id).await?;
        rpc.call("turn/interrupt",json!({"threadId":id,"turnId":turn})).await
    }
    pub async fn settings(&self, session: &str, settings: Value) -> Result<Value> {
        self.thread(session).await?;
        let rpc=self.connect(false).await?;
        let mut params=settings.as_object().context("invalid Codex settings")?.clone();
        params.insert("threadId".into(),json!(super::session::native_id(session).unwrap()));
        rpc.call("thread/settings/update",Value::Object(params)).await
    }
    pub async fn action(&self, session: &str, method: &str) -> Result<Value> {
        if method=="thread/unarchive" {
            if !self.hosts(session) {bail!("{}",super::live::WATCHED);}
        } else {self.thread(session).await?;}
        self.connect(false).await?.call(method,json!({"threadId":super::session::native_id(session).unwrap()})).await
    }
    async fn active_turn(&self, rpc: &Rpc, id: &str) -> Result<String> {
        let turns=rpc.call("thread/turns/list",json!({"threadId":id,"limit":1,"sortDirection":"desc"})).await?;
        turns["data"].as_array().and_then(|rows|rows.iter().find(|t|t["status"]=="inProgress"))
            .and_then(|t|t["id"].as_str()).map(str::to_owned).context("no active Codex turn remains; refresh before sending the action")
    }
    pub async fn refresh(&self) -> Result<()> {
        let rpc=self.connect(false).await?;
        let mut cursor=Value::Null;
        loop {
            let loaded=rpc.call("thread/loaded/list",json!({"limit":100,"cursor":cursor})).await?;
            for id in loaded["data"].as_array().context("invalid loaded Codex thread list")? {
                let id=id.as_str().context("invalid Codex thread identity")?;
                let result=rpc.call("thread/read",json!({"threadId":id,"includeTurns":false})).await?;
                let thread=&result["thread"];
                if !thread["parentThreadId"].is_null() || thread["threadSource"]=="thread_title" {continue;}
                if !locked(&rpc.observed).subscribed.contains(id) {
                    let resumed=rpc.call("thread/resume",json!({"threadId":id,"excludeTurns":true})).await?;
                    self.remember(&rpc,&resumed["thread"])?;
                } else {self.remember(&rpc,thread)?;}
            }
            cursor=loaded["nextCursor"].clone();
            if cursor.is_null() {break;}
        }
        Ok(())
    }
    pub fn snapshot(&self, session: &str) -> Option<(Value,bool)> {
        let id=super::session::native_id(session)?;
        if let Ok(held)=self.connection.try_lock() {
            if let Some(rpc)=held.as_ref() {
                return locked(&rpc.observed).threads.get(id).cloned().map(|t|(t,rpc.alive()));
            }
        }
        std::fs::read(self.options.root.join("threads.json")).ok()
            .and_then(|raw|serde_json::from_slice::<BTreeMap<String,Value>>(&raw).ok())
            .and_then(|threads|threads.get(id).cloned()).map(|t|(t,false))
    }
    pub fn known(&self) -> Vec<String> {
        if let Ok(held)=self.connection.try_lock() {
            if let Some(rpc)=held.as_ref() { return locked(&rpc.observed).threads.keys().map(|id|super::session::router_id(id)).collect(); }
        }
        std::fs::read(self.options.root.join("threads.json")).ok()
            .and_then(|raw|serde_json::from_slice::<BTreeMap<String,Value>>(&raw).ok())
            .map(|threads|threads.keys().map(|id|super::session::router_id(id)).collect()).unwrap_or_default()
    }
    pub fn activity(&self, session: &str, mut activity: crate::activity::Activity) -> crate::activity::Activity {
        let Some((thread,alive))=self.snapshot(session) else {return activity};
        let status=thread["status"]["type"].as_str().unwrap_or("notLoaded");
        let open=alive && status!="notLoaded";
        activity.open=Some(open);activity.source="codex-app-server";
        activity.kind=if !open {crate::activity::Kind::Closed} else {match status {
            "active"=>crate::activity::Kind::Generating,
            "systemError"=>crate::activity::Kind::Failed,
            _=>crate::activity::Kind::Idle,
        }};
        activity.mid_turn=Some(open && status=="active");
        activity.detail=if !alive {"Managed Codex is disconnected"} else if !open {"Codex thread is not loaded"} else if status=="active" {"Working"} else {"Waiting for you"}.into();
        if let Ok(held)=self.connection.try_lock() {
            if let Some(rpc)=held.as_ref().filter(|r|r.alive()) {
                if let Some(pending)=rpc.pending(super::session::native_id(session).unwrap()).first() {
                    activity.kind=crate::activity::Kind::Waiting;
                    let question=pending.method=="item/tool/requestUserInput";
                    let tool=if question {"AskUserQuestion"} else if pending.method=="item/fileChange/requestApproval" {"Edit"} else {"Bash"};
                    activity.detail=if question {"Waiting for your answer"} else {"Waiting for approval"}.into();
                    activity.waiting_on=Some(crate::activity::WaitingOn {kind:if question {"question"} else {"permission"},tool:Some(tool.into()),request_id:Some(pending.key.clone()),tool_use_id:pending.params["itemId"].as_str().map(str::to_owned),summary:pending.params["reason"].as_str().map(str::to_owned)});
                }
            }
        }
        activity
    }
}

pub fn host() -> Option<&'static Host> {
    static HOST: OnceLock<Host> = OnceLock::new();
    if let Some(host)=HOST.get() { return Some(host); }
    let options=Options::here().ok()?;
    Some(HOST.get_or_init(||Host::new(options)))
}

pub async fn keep_host() {
    loop {
        if let Some(host)=host().filter(|h|h.options.root.join("options.json").exists()) {
            if let Err(error)=host.refresh().await { tracing::debug!(%error,"managed Codex is unavailable"); }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// A separate long-lived process owns the backend across router/agent/frontend exits.
/// The private file lock makes racing launchers share exactly one backend.
pub async fn supervise(path: &Path) -> Result<()> {
    use ring::rand::SecureRandom;
    let options:Options=serde_json::from_slice(&std::fs::read(path)?)?;
    private_dir(&options.root)?;
    let lock_path=options.root.join("supervisor.lock");
    if std::fs::symlink_metadata(&lock_path).is_ok_and(|m|m.file_type().is_symlink()) { bail!("runtime lock is a symlink"); }
    let mut open=std::fs::OpenOptions::new();open.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt;open.mode(0o600); }
    let lock=open.open(lock_path)?;
    if lock.try_lock().is_err() { return Ok(()); }
    let control=tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mut secret=[0u8;32];ring::rand::SystemRandom::new().fill(&mut secret).map_err(|_|anyhow!("system randomness unavailable"))?;
    let secret=secret.iter().map(|b|format!("{b:02x}")).collect::<String>();
    let verifier=ring::hmac::Key::new(ring::hmac::HMAC_SHA256,b"agent-router-codex-supervisor");
    let expected=ring::hmac::sign(&verifier,secret.as_bytes());
    let control_path=options.root.join("control.json");
    private_file(&control_path,&serde_json::to_vec(&json!({"address":control.local_addr()?.to_string(),"token":secret}))?)?;
    let log=std::fs::OpenOptions::new().create(true).append(true).open(options.root.join("app-server.log"))?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;log.set_permissions(std::fs::Permissions::from_mode(0o600))?;}
    let mut child=options.command().args(["-c","features.code_mode_host=true","app-server","--listen"])
        .arg(format!("unix://{}",options.socket().display()))
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(log)
        .kill_on_drop(true).spawn()?;
    let terminate=async {
        #[cfg(unix)] { tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap().recv().await; }
        #[cfg(windows)] std::future::pending::<()>().await;
    };
    tokio::pin!(terminate);
    loop {
        tokio::select! {
            status=child.wait()=>{if !status?.success() {bail!("managed Codex exited; inspect app-server.log");}break;}
            _=tokio::signal::ctrl_c()=>{child.kill().await?;break;}
            _=&mut terminate=>{child.kill().await?;break;}
            accepted=control.accept()=> {
                let (socket,_)=accepted?;
                let mut stream=BufReader::new(socket);
                let mut line=Vec::new();
                if !matches!(tokio::time::timeout(Duration::from_secs(2),tokio::io::AsyncReadExt::take(&mut stream,1025).read_until(b'\n',&mut line)).await,Ok(Ok(n)) if n>0 && n<=1024) {continue;}
                let Ok(request)=serde_json::from_slice::<Value>(&line) else {continue};
                if !request["token"].as_str().is_some_and(|token|ring::hmac::verify(&verifier,token.as_bytes(),expected.as_ref()).is_ok()) {continue;}
                if request["action"]=="stop" {
                    child.kill().await?;child.wait().await?;
                    stream.get_mut().write_all(b"{\"ok\":true}\n").await?;
                    break;
                }
            }
        }
    }
    let _=std::fs::remove_file(control_path);
    Ok(())
}

pub async fn stop(options: &Options) -> Result<()> {
    let control:Value=serde_json::from_slice(&std::fs::read(options.root.join("control.json"))?)?;
    let address:std::net::SocketAddr=control["address"].as_str().context("no supervisor address")?.parse()?;
    if address.ip()!=std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {bail!("supervisor control must stay on loopback");}
    let mut socket=tokio::net::TcpStream::connect(address).await?;
    let mut request=serde_json::to_vec(&json!({"token":control["token"],"action":"stop"}))?;request.push(b'\n');
    socket.write_all(&request).await?;
    let mut reply=String::new();
    tokio::time::timeout(WAIT,BufReader::new(socket).read_line(&mut reply)).await??;
    if serde_json::from_str::<Value>(&reply)?["ok"]!=true {bail!("managed Codex stop was not acknowledged");}
    Ok(())
}

async fn stdio_proxy(options: &Options) -> Result<()> {
    let mut child=options.command().args(["app-server","proxy","--sock"]).arg(options.socket())
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit()).kill_on_drop(true).spawn()?;
    let websocket=socket(&mut child).await?;
    let (mut writer,mut reader)=websocket.split();
    let mut input=BufReader::new(tokio::io::stdin());let mut output=tokio::io::stdout();
    let mut line=Vec::new();
    loop {
        let mut limited=tokio::io::AsyncReadExt::take(&mut input,(MAX_LINE+1-line.len()) as u64);
        tokio::select! {
            read=limited.read_until(b'\n',&mut line)=> {
                let n=read?;if n==0 && line.is_empty() {writer.close().await?;break;}
                if line.len()>MAX_LINE || line.last()!=Some(&b'\n') {bail!("invalid Codex frontend JSON line");}
                line.pop();let text=String::from_utf8(std::mem::take(&mut line))?;
                // Validate framing, preserving every native method, ID and field.
                serde_json::from_str::<Value>(&text)?;
                writer.send(Message::Text(text.into())).await?;
            }
            message=reader.next()=>match message {
                Some(Ok(Message::Text(text)))=> {output.write_all(text.as_bytes()).await?;output.write_all(b"\n").await?;output.flush().await?;}
                Some(Ok(Message::Close(_)))|None=>break,
                Some(Ok(Message::Ping(_)|Message::Pong(_)))=>{},
                Some(Ok(_))=>bail!("unexpected native socket frame"),
                Some(Err(e))=>return Err(e.into()),
            }
        }
    }
    Ok(())
}

pub async fn cli(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("models")=>super::catalogue::cli(args).await,
        Some("supervise") if args.len()==2=>supervise(Path::new(&args[1])).await,
        Some("configure") if args.len()==2=> {
            let home=crate::accounts::codex_home()?.canonicalize()?;
            let options=Options {root:home.join("agent-router-runtime"),backend:Path::new(&args[1]).canonicalize()?,codex_home:home,router_binary:std::env::current_exe()?};
            options.prepare().await?;
            options.install_launcher()?;
            println!("{}",options.launcher().display());
            Ok(())
        }
        Some("restart")|Some("stop") if args.len()==1=> {
            let host=host().context("no managed Codex runtime")?;
            host.refresh().await?;
            for id in host.known() {
                if host.snapshot(&id).is_some_and(|(t,alive)|alive && t["status"]["type"]=="active") {bail!("finish active Codex turns before stopping or restarting its managed backend");}
            }
            stop(&host.options).await?;
            *host.connection.lock().await=None;
            if args[0]=="restart" {host.connect(true).await?;}
            Ok(())
        }
        Some("proxy")|Some("run")=> {
            let host=host().context("configure the managed Codex runtime first")?;
            host.connect(true).await?;
            if args[0]=="proxy" {return stdio_proxy(&host.options).await;}
            let mut command=host.options.command();
            command.stdin(std::process::Stdio::inherit()).stdout(std::process::Stdio::inherit()).stderr(std::process::Stdio::inherit());
            command.arg("--remote").arg(format!("unix://{}",host.options.socket().display())).args(&args[1..]);
            let status=command.status().await?;
            if !status.success() { std::process::exit(status.code().unwrap_or(1)); }
            Ok(())
        }
        _=>bail!("usage: agent-router codex models refresh|restore | configure /path/to/codex | run [CODEX_OPTIONS] | proxy"),
    }
}

/// The editor's configured executable remains a CLI-shaped launcher. Native
/// app-server invocations use its stdio proxy; terminal invocations attach to the
/// same backend with Codex's own --remote transport. Version/schema/help commands
/// go straight to the real binary without starting a backend.
pub async fn adapter(args: &[String]) -> Result<()> {
    let options=Options::here()?;
    if args.iter().any(|a|matches!(a.as_str(),"--version"|"-V"|"--help"|"-h"|"generate-ts"|"generate-json-schema")) {
        let status=options.command().args(args).stdin(std::process::Stdio::inherit()).stdout(std::process::Stdio::inherit()).stderr(std::process::Stdio::inherit()).status().await?;
        std::process::exit(status.code().unwrap_or(1));
    }
    if args.iter().any(|a|a=="app-server") {
        let mut index=0;
        while index<args.len() {
            match args[index].as_str() {
                "app-server"|"--analytics-default-enabled"|"--stdio"=>index+=1,
                "-c"|"--config" if args.get(index+1).is_some_and(|v|v=="features.code_mode_host=true")=>index+=2,
                "--listen" if args.get(index+1).is_some_and(|v|v=="stdio://")=>index+=2,
                _=>bail!("this Codex editor launch is not qualified for the managed stdio adapter: {}",args[index]),
            }
        }
        cli(&["proxy".to_owned()]).await
    } else {
        let mut run=vec!["run".to_owned()];run.extend_from_slice(args);cli(&run).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture(tag: &str) -> (Host, Arc<Rpc>,String) {
        let root=std::env::temp_dir().join(format!("ar-codex-rpc-{tag}-{}-{}",std::process::id(),CONNECTION.fetch_add(1,Ordering::Relaxed)));
        let options=Options {root:root.clone(),backend:PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex-runtime.py"),codex_home:root.clone(),router_binary:std::env::current_exe().unwrap()};
        options.prepare().await.unwrap();
        let host=Host::new(options);
        let rpc=Rpc::open(&host.options,true).await.unwrap();
        *host.connection.lock().await=Some(rpc.clone());
        let session=host.start(&std::env::temp_dir(),"fixture-model").await.unwrap();
        (host,rpc,session)
    }

    #[tokio::test]
    async fn replies_match_ids_even_when_the_native_peer_reorders_them() {
        let (host,rpc,_)=fixture("reorder").await;
        let first={let rpc=rpc.clone();tokio::spawn(async move {rpc.call("test/deferred",json!({})).await.unwrap()})};
        for _ in 0..50 {
            if !locked(&rpc.replies).is_empty() {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let second=rpc.call("test/reverse",json!({})).await.unwrap();
        assert_eq!(second["method"],"test/reverse");assert_eq!(first.await.unwrap()["method"],"test/deferred");
        assert!(locked(&rpc.replies).is_empty());
        assert!(host.hosts("codex:thread-1"));
    }

    #[tokio::test]
    async fn disconnects_fail_unacknowledged_actions_and_clear_approval_ownership() {
        let (_,rpc,session)=fixture("disconnect").await;
        rpc.call("test/approval",json!({"threadId":super::super::session::native_id(&session).unwrap()})).await.unwrap();
        assert_eq!(rpc.pending("thread-1").len(),1);
        let error=rpc.call("test/disconnect",json!({})).await.unwrap_err();
        assert!(error.to_string().contains("not acknowledged"));
        assert!(!rpc.alive());assert!(rpc.pending("thread-1").is_empty());
        assert!(rpc.call("thread/start",json!({})).await.is_err());
    }

    #[tokio::test]
    async fn approvals_are_bound_to_connection_and_thread_and_wait_for_resolution() {
        let (_,rpc,_)=fixture("approval").await;
        rpc.call("test/foreign-approval",json!({"threadId":"unmanaged"})).await.unwrap();
        assert!(rpc.pending("unmanaged").is_empty());
        rpc.call("test/approval",json!({"threadId":"thread-1"})).await.unwrap();
        let key=rpc.pending("thread-1")[0].key.clone();
        assert!(rpc.answer("other-thread",&key,json!({"decision":"accept"})).await.is_err());
        assert!(rpc.answer("thread-1","previous-connection:90",json!({"decision":"accept"})).await.is_err());
        rpc.answer("thread-1",&key,json!({"decision":"decline"})).await.unwrap();
        assert!(rpc.pending("thread-1").is_empty());
        assert!(rpc.answer("thread-1",&key,json!({"decision":"accept"})).await.is_err());
    }

    #[tokio::test]
    async fn native_refusals_and_malformed_frames_never_become_success() {
        let (host,rpc,session)=fixture("refusal").await;
        let error=host.settings(&session,json!({"model":"refuse"})).await.unwrap_err();
        assert_eq!(error.to_string(),"model refused by native peer");
        assert!(host.thread("codex:unmanaged").await.unwrap_err().to_string().contains("watched"));
        assert!(rpc.call("test/malformed",json!({})).await.is_err());
        assert!(!rpc.alive());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_files_are_private_and_symlinks_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (host,_,_)=fixture("files").await;
        assert_eq!(std::fs::metadata(&host.options.root).unwrap().permissions().mode() & 0o777,0o700);
        for name in ["options.json","threads.json"] {assert_eq!(std::fs::metadata(host.options.root.join(name)).unwrap().permissions().mode() & 0o777,0o600);}
        let outside=host.options.root.join("outside");std::fs::write(&outside,"preserved").unwrap();
        let link=host.options.root.join("link");std::os::unix::fs::symlink(&outside,&link).unwrap();
        assert!(private_file(&link,b"changed").is_err());assert_eq!(std::fs::read_to_string(outside).unwrap(),"preserved");
    }
}

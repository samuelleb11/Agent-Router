//! The live view for native Codex sessions. Rollouts are safe to watch while their
//! terminal or IDE owns execution; control requires a separately hosted app-server.

use std::convert::Infallible;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::session;
use crate::live::StreamStatus;

pub const WATCHED: &str = "This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it.";

pub fn is_codex(id: &str) -> bool { id.starts_with("codex:") }

pub fn unsupported() -> Response {
    (StatusCode::CONFLICT, Json(json!({ "ok": false, "reason": WATCHED }))).into_response()
}

pub fn watched_prompt() -> Response {
    // Native rollouts do not persist a current approval channel. Reporting an old
    // request as pending would offer a control the router cannot actually apply.
    Json(json!({ "pending": false, "count": 0 })).into_response()
}

fn failed(error: anyhow::Error) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"ok":false,"reason":error.to_string()}))).into_response()
}

fn driven(session: &str) -> Option<&'static super::runtime::Host> {
    super::runtime::host().filter(|h|h.hosts(session))
}

pub async fn send(session: &str, text: &str) -> Response {
    let Some(host)=driven(session) else {return unsupported()};
    match host.send(session,text).await {
        Ok(result)=>Json(json!({"ok":true,"via":"codex-app-server","acknowledged":true,"result":result})).into_response(),
        Err(e)=>failed(e),
    }
}

pub async fn interrupt(session: &str) -> Response {
    let Some(host)=driven(session) else {return unsupported()};
    match host.interrupt(session).await {Ok(result)=>Json(json!({"ok":true,"acknowledged":true,"result":result})).into_response(),Err(e)=>failed(e)}
}

pub async fn settings(session: &str, settings: serde_json::Value) -> Response {
    let Some(host)=driven(session) else {return unsupported()};
    match host.settings(session,settings).await {Ok(result)=>Json(json!({"ok":true,"acknowledged":true,"result":result})).into_response(),Err(e)=>failed(e)}
}

pub async fn mode(session: &str, mode: &str) -> Response {
    let Some(host)=driven(session) else {return unsupported()};
    let mut settings=match mode {
        "default"|"manual"=>json!({"approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":[],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}),
        "acceptEdits"|"auto"=>json!({"approvalPolicy":"on-request","sandboxPolicy":{"type":"workspaceWrite","writableRoots":[],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}),
        "bypassPermissions"|"yolo"=>json!({"approvalPolicy":"never","sandboxPolicy":{"type":"dangerFullAccess"}}),
        "plan"=> {
            let thread=match host.thread(session).await {Ok(t)=>t,Err(e)=>return failed(e)};
            json!({"approvalPolicy":"on-request","sandboxPolicy":{"type":"readOnly"},"collaborationMode":{"mode":"plan","settings":{"model":thread["model"],"reasoning_effort":thread["reasoningEffort"],"developer_instructions":null}}})
        }
        _=>return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Codex modes are default, acceptEdits, plan and bypassPermissions."}))).into_response(),
    };
    if mode!="plan" {
        let thread=match host.thread(session).await {Ok(t)=>t,Err(e)=>return failed(e)};
        settings["collaborationMode"]=json!({"mode":"default","settings":{"model":thread["model"],"reasoning_effort":thread["reasoningEffort"],"developer_instructions":null}});
    }
    match host.settings(session,settings).await {Ok(result)=>Json(json!({"ok":true,"mode":mode,"acknowledged":true,"result":result})).into_response(),Err(e)=>failed(e)}
}

pub async fn action(session: &str, method: &str) -> Response {
    let Some(host)=driven(session) else {return unsupported()};
    match host.action(session,method).await {Ok(result)=>Json(json!({"ok":true,"acknowledged":true,"result":result})).into_response(),Err(e)=>failed(e)}
}

fn prompt_view(p: &super::runtime::Pending) -> Option<serde_json::Value> {
    let (tool,question,input)=match p.method.as_str() {
        "item/commandExecution/requestApproval"=>("Bash",false,json!({"command":p.params["command"],"cwd":p.params["cwd"]})),
        "item/fileChange/requestApproval"=>("Edit",false,p.params.clone()),
        "item/permissions/requestApproval"=>("Permissions",false,p.params["permissions"].clone()),
        "item/tool/requestUserInput"=> {
            let questions=p.params["questions"].as_array()?.iter().map(|q|json!({"question":q["question"],"header":q["header"],"multiSelect":false,"options":q["options"].as_array().map(|opts|opts.iter().map(|o|json!({"label":o["label"],"description":o["description"]})).collect::<Vec<_>>()).unwrap_or_default()})).collect::<Vec<_>>();
            ("AskUserQuestion",true,json!({"questions":questions}))
        }
        _=>return None,
    };
    Some(json!({"pending":true,"request_id":p.key,"tool":tool,"question":question,"display_name":tool,"description":p.params["reason"],"reason":p.params["reason"],"input":input,"suggestions":[]}))
}

pub async fn prompt(session: &str) -> Response {
    let Some(host)=driven(session) else {return watched_prompt()};
    let rpc=match host.connect(false).await {Ok(r)=>r,Err(e)=>return failed(e)};
    let prompts=rpc.pending(super::session::native_id(session).unwrap()).iter().filter_map(prompt_view).collect::<Vec<_>>();
    match prompts.first().cloned() {
        Some(mut first)=>{first["count"]=json!(prompts.len());first["prompts"]=json!(prompts);Json(first).into_response()}
        None=>watched_prompt(),
    }
}

pub async fn answer(body: &crate::live::AnswerBody) -> Response {
    let Some(host)=driven(&body.session) else {return unsupported()};
    let rpc=match host.connect(false).await {Ok(r)=>r,Err(e)=>return failed(e)};
    let native=super::session::native_id(&body.session).unwrap();
    let pending=rpc.pending(native);
    let chosen=match body.request_id.as_deref() {
        Some(id)=>pending.iter().find(|p|p.key==id),
        None if pending.len()==1=>pending.first(),
        None=>None,
    };
    let Some(p)=chosen else {return (StatusCode::CONFLICT,Json(json!({"ok":false,"reason":"Refresh and choose the Codex request that is still waiting."}))).into_response()};
    if body.suggestion.is_some() {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Codex approval suggestions must be selected in its native client."}))).into_response()}
    let result=match p.method.as_str() {
        "item/commandExecution/requestApproval"|"item/fileChange/requestApproval"=> {
            let Some(allow)=body.allow else {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"This request needs an explicit approval or refusal."}))).into_response()};
            let decision=if allow {"accept"} else {"decline"};
            if p.params["availableDecisions"].as_array().is_some_and(|opts|!opts.contains(&json!(decision))) {return (StatusCode::CONFLICT,Json(json!({"ok":false,"reason":"Codex did not offer that approval decision."}))).into_response()}
            json!({"decision":decision})
        }
        "item/permissions/requestApproval"=> {
            let Some(allow)=body.allow else {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"This request needs an explicit approval or refusal."}))).into_response()};
            json!({"permissions":if allow {p.params["permissions"].clone()} else {json!({})},"scope":"turn"})
        }
        "item/tool/requestUserInput"=> {
            let Some(questions)=p.params["questions"].as_array() else {return failed(anyhow::anyhow!("Codex sent invalid questions"))};
            let mut answers=serde_json::Map::new();
            for q in questions {
                let Some(answer)=q["question"].as_str().and_then(|text|body.answers.get(text)) else {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Answer every Codex question before submitting."}))).into_response()};
                if q["options"].as_array().is_some_and(|opts|!opts.iter().any(|o|o["label"]==answer.as_str())) && q["isOther"]!=true {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Choose an answer offered by Codex."}))).into_response()}
                let Some(id)=q["id"].as_str() else {return failed(anyhow::anyhow!("Codex question has no id"))};
                answers.insert(id.to_owned(),json!({"answers":[answer]}));
            }
            json!({"answers":answers})
        }
        _=>return (StatusCode::CONFLICT,Json(json!({"ok":false,"reason":"Answer this request in its native Codex client."}))).into_response(),
    };
    match rpc.answer(native,&p.key,result).await {
        Ok(())=>Json(json!({"ok":true,"request_id":p.key,"acknowledged":true})).into_response(),
        Err(e)=>failed(e),
    }
}

#[derive(serde::Deserialize)]
pub struct StartBody {
    pub cwd: String,
    pub model: String,
    #[serde(default)] pub client: Option<String>,
    #[serde(default)] pub machine: Option<String>,
}

pub async fn start_route(crate::refusal::JsonBody(body): crate::refusal::LiveBody<StartBody>) -> Response {
    match body.client.as_deref().unwrap_or("kimi-code") {
        "kimi-code"=>crate::kimi::live::start(crate::kimi::live::host(),&crate::kimi::live::StartBody {cwd:body.cwd,model:body.model,machine:body.machine}).await,
        "codex"=> {
            let Some(host)=super::runtime::host() else {return failed(anyhow::anyhow!("Configure a native Codex backend on this machine first."))};
            match host.start(std::path::Path::new(&body.cwd),&body.model).await {
                Ok(id)=>Json(json!({"ok":true,"session":id,"client":"codex","acknowledged":true})).into_response(),
                Err(e)=>failed(e),
            }
        }
        _=>(StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Choose Codex or Kimi Code for a managed session."}))).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ResumeBody {pub session:String}

pub async fn resume_route(crate::refusal::JsonBody(body):crate::refusal::LiveBody<ResumeBody>) -> Response {
    if !is_codex(&body.session) {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"reason":"Resume is for a native Codex thread."}))).into_response()}
    let Some(host)=super::runtime::host() else {return failed(anyhow::anyhow!("Configure a native Codex backend first."))};
    match host.resume(&body.session).await {
        Ok(result)=>Json(json!({"ok":true,"session":body.session,"acknowledged":true,"result":result})).into_response(),Err(e)=>failed(e),
    }
}

pub async fn compact_route(crate::refusal::JsonBody(body):crate::refusal::LiveBody<ResumeBody>) -> Response {
    action(&body.session,"thread/compact/start").await
}

pub fn stream(session_id: &str) -> Response {
    let id = session_id.to_owned();
    let s = async_stream::stream! {
        let mut sent = 0usize;
        let mut last_status: Option<StreamStatus> = None;
        let mut last_sent = Instant::now();
        let mut questions=std::collections::HashSet::new();
        loop {
            let found = {
                let id = id.clone();
                tokio::task::spawn_blocking(move || session::find(&id)).await.ok().flatten()
            };
            let Some(session) = found else {
                yield Ok::<_, Infallible>(Event::default().event("error").data("no such session"));
                return;
            };
            let feed = session.feed();
            if feed.len() < sent { sent = 0; }
            for item in &feed[sent..] {
                if let Ok(json) = serde_json::to_string(item) { yield Ok(Event::default().data(json)); }
            }
            sent = feed.len();
            if let Some(host)=super::runtime::host().filter(|h|h.hosts(&id)) {
                if let Ok(rpc)=host.connect(false).await {
                    for pending in rpc.pending(super::session::native_id(&id).unwrap()) {
                        if pending.method=="item/tool/requestUserInput" && questions.insert(pending.key.clone()) {
                            if let Some(view)=prompt_view(&pending) {
                                let item=crate::live::Item {t:None,role:"assistant".into(),kind:"ask".into(),name:Some("AskUserQuestion".into()),text:None,ask:Some(view["input"].clone()),dur_ms:None,sidechain:false};
                                if let Ok(json)=serde_json::to_string(&item) {yield Ok(Event::default().data(json));}
                            }
                        }
                    }
                }
            }
            let now = crate::db::now_ms();
            let status = StreamStatus::of_activity(&session.activity(now), now);
            if crate::live::status_due(last_status.as_ref(), &status, last_sent.elapsed()) {
                if let Ok(json) = serde_json::to_string(&status) { yield Ok(Event::default().event("status").data(json)); }
                last_status = Some(status); last_sent = Instant::now();
            }
            // Drop the snapshot before the next fold, letting its memo append in place.
            drop(session);
            tokio::time::sleep(crate::live::POLL).await;
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn watched_codex_control_says_where_the_session_can_be_driven() {
        assert!(is_codex("codex:thread-1"));
        assert!(!is_codex("session_kimi"));
        let response = unsupported();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["reason"], WATCHED);
    }
}

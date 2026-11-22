use crate::db::Database;
use crate::models::{Calendar, Event, Todo};
use crate::sync::ical;
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashSet};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::time::Duration;
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader as XmlReader;

#[derive(Debug, Clone)]
pub struct SyncAccountConfig {
    pub provider: String,
    pub server_url: String,
    pub username: String,
    pub token_or_password: String,
}

pub struct SyncEngine;

impl SyncEngine {
    fn build_agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(20))
            .timeout_write(Duration::from_secs(20))
            .build()
    }

    fn apply_auth(req: ureq::Request, cfg: &SyncAccountConfig) -> ureq::Request {
        if !cfg.username.trim().is_empty() || !cfg.token_or_password.is_empty() {
            let raw = format!("{}:{}", cfg.username.trim(), cfg.token_or_password);
            let encoded = base64_encode(raw.as_bytes());
            req.set("Authorization", &format!("Basic {}", encoded))
        } else {
            req
        }
    }

    /// 1. 标准 CalDAV 自动发现：探测并获取用户的 Calendar Home 集合或具体日历 URL
    pub fn discover_calendars(cfg: &SyncAccountConfig) -> Result<Vec<String>> {
        let base_url = cfg.server_url.trim().trim_end_matches('/');
        let agent = Self::build_agent();

        let propfind_body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
    <c:calendar-home-set/>
  </d:prop>
</d:propfind>"#;

        let req = Self::apply_auth(agent.request("PROPFIND", base_url), cfg)
            .set("Depth", "1")
            .set("Content-Type", "application/xml; charset=utf-8");

        match req.send_string(propfind_body) {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.into_string().unwrap_or_default();
                
                if status == 207 || (200..300).contains(&status) {
                    let mut found_urls = Vec::new();
                    found_urls.push(base_url.to_string());
                    
                    for line in body.lines() {
                        if line.contains("<d:href>") || line.contains("<href>") {
                            if let Some(start) = line.find('>') {
                                if let Some(end) = line.find("</") {
                                    let href = line[start + 1..end].trim();
                                    if !href.is_empty() {
                                        let full_url = if href.starts_with("http") {
                                            href.to_string()
                                        } else if href.starts_with('/') {
                                            // 纯标准库解析 scheme 和 host 拼接，无需依赖外部 url crate
                                            if let Some(idx) = base_url.find("://") {
                                                let rest = &base_url[idx + 3..];
                                                let host_end = rest.find('/').unwrap_or(rest.len());
                                                let origin = &base_url[..idx + 3 + host_end];
                                                format!("{}{}", origin, href)
                                            } else {
                                                format!("{}{}", base_url, href)
                                            }
                                        } else {
                                            format!("{}/{}", base_url, href)
                                        };
                                        if !found_urls.contains(&full_url) {
                                            found_urls.push(full_url);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(found_urls)
                } else {
                    Ok(vec![base_url.to_string()])
                }
            }
            Err(_) => {
                Ok(vec![base_url.to_string()])
            }
        }
    }

    /// 真实检查 CalDAV / WebDAV 连接配置
    pub fn test_connection(cfg: &SyncAccountConfig) -> Result<String> {
        let url = cfg.server_url.trim();
        if url.is_empty() {
            bail!("服务器地址或 Endpoint URL 不能为空");
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!("服务器地址格式错误，必须以 http:// 或 https:// 开头");
        }

        let agent = Self::build_agent();
        let propfind_body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
  </d:prop>
</d:propfind>"#;

        let req = Self::apply_auth(agent.request("PROPFIND", url), cfg)
            .set("Depth", "0")
            .set("Content-Type", "application/xml; charset=utf-8");

        match req.send_string(propfind_body) {
            Ok(resp) => {
                let status = resp.status();
                if status == 207 || (200..300).contains(&status) {
                    Ok(format!(
                        "[{}] CalDAV 服务器响应正常 (HTTP {})，自动发现验证通过 ({})",
                        cfg.provider, status, url
                    ))
                } else {
                    bail!("服务器返回非预期状态码 HTTP {}", status);
                }
            }
            Err(ureq::Error::Status(code, resp)) => match code {
                401 => bail!("身份验证失败 (HTTP 401 Unauthorized)：请检查账号或密码是否正确"),
                403 => bail!("访问被拒绝 (HTTP 403 Forbidden)：当前账号无权访问该路径"),
                404 => bail!("路径不存在 (HTTP 404 Not Found)：请检查服务器 Endpoint URL"),
                405 => {
                    let fallback = Self::apply_auth(agent.head(url), cfg).call();
                    match fallback {
                        Ok(r) => Ok(format!(
                            "[{}] 服务器连通正常 (HTTP {})，支持 WebDAV/CalDAV 访问",
                            cfg.provider,
                            r.status()
                        )),
                        Err(_) => bail!("服务器不支持 PROPFIND 且验证失败 (HTTP 405)"),
                    }
                }
                _ => bail!("服务器返回错误状态码 HTTP {} ({})", code, resp.status_text()),
            },
            Err(ureq::Error::Transport(t)) => {
                bail!("网络传输错误，无法连接到服务器：{}", t);
            }
        }
    }

    pub fn sync_all(
        cfg: &SyncAccountConfig, db: &mut Database, calendars: &[Calendar],
        local_events: &[Event], local_todos: &[Todo],
    ) -> Result<String> {
        Self::sync_with_mode(cfg, db, calendars, local_events, local_todos, SyncMode::Merge)
    }

    pub fn sync_with_mode(
        cfg: &SyncAccountConfig, db: &mut Database, calendars: &[Calendar],
        local_events: &[Event], local_todos: &[Todo], mode: SyncMode,
    ) -> Result<String> {
        let target = cfg.server_url.trim().trim_end_matches('/');
        if target.is_empty() { bail!("请填写具体的 CalDAV 日历集合 URL"); }
        Self::test_connection(cfg)?;
        let agent = Self::build_agent();
        let calendar_id = calendars.iter().find(|c| c.is_visible).or_else(|| calendars.first())
            .map(|c| c.id.clone()).unwrap_or_default();
        let state_key = format!("{}|{}|{}", cfg.provider, cfg.username.trim(), target);
        let baseline = db.get_caldav_sync_state(&state_key)?
            .map(|s| serde_json::from_str::<Baseline>(&s)).transpose()?
            .unwrap_or_default();

        // RFC 4791 calendar-query component filters are conjunctive. Asking for
        // VEVENT and VTODO as sibling filters in one query can therefore match
        // nothing on compliant servers. Query each component type independently.
        let xml_events = fetch_calendar_report(cfg, &agent, target, "VEVENT")?;
        let xml_todos = fetch_calendar_report(cfg, &agent, target, "VTODO")?;
        if !xml_events.contains("multistatus") || !xml_todos.contains("multistatus") {
            bail!("服务器未返回有效 DAV multistatus，已停止同步以保护数据");
        }
        let dav_responses = parse_dav_responses(&xml_events)?
            .into_iter()
            .chain(parse_dav_responses(&xml_todos)?)
            .collect::<Vec<_>>();
        let mut remote: BTreeMap<String, Remote> = BTreeMap::new();
        let mut parsed_resources = 0usize;
        let mut missing_data_responses = 0usize;
        for (response_index, response) in dav_responses.iter().enumerate() {
            let href = response.href.as_deref().unwrap_or_default();
            if href.trim().is_empty() {
                bail!("CalDAV REPORT 第 {} 个 DAV response 缺少有效 href（包含 calendar-data: {}，ETag: {}）；已停止同步以保护远程日程。请检查服务器返回的多状态 XML。",
                    response_index + 1, response.calendar_data.is_some(), response.etag.as_deref().unwrap_or("无"));
            }
            let etag = response.etag.clone();
            let Some(raw) = response.calendar_data.as_deref() else { missing_data_responses += 1; continue; };
            parsed_resources += 1;
            let data = raw.to_string();
            let (events, todos) = ical::import_from_ics_with_progress(&data, &calendar_id, |_,_|{})
                .context("云端 iCalendar 解析失败；同步已中止")?;
            if events.len() + todos.len() != 1 {
                bail!("资源 {} 包含 {} 个 VEVENT/VTODO；当前本地模型不能无损改写多组件资源，已中止以防丢失重复日程或其他组件", href, events.len() + todos.len());
            }
            let href = resolve_href(target, &xml_unescape(&href));
            for mut v in events {
                if let Some(local) = local_events.iter().find(|x| x.id == v.id) { v.calendar_id = local.calendar_id.clone(); }
                remote.insert(format!("E:{}",v.id), Remote { item: Item::Event(v), href: href.clone(), etag: etag.clone() });
            }
            for mut v in todos {
                if let Some(local) = local_todos.iter().find(|x| x.id == v.id) { v.calendar_id = local.calendar_id.clone(); }
                remote.insert(format!("T:{}",v.id), Remote { item: Item::Todo(v), href: href.clone(), etag: etag.clone() });
            }
        }
        if missing_data_responses > 0 || (!dav_responses.is_empty() && parsed_resources == 0) {
            bail!("服务器有 DAV response 缺少 calendar-data；可能是权限、属性查询或服务器错误，已停止同步以防误删");
        }
        let mut local = BTreeMap::new();
        for v in local_events { local.insert(format!("E:{}",v.id), Item::Event(v.clone())); }
        for v in local_todos { local.insert(format!("T:{}",v.id), Item::Todo(v.clone())); }
        let mut keys: HashSet<String> = baseline.items.keys().cloned().collect();
        keys.extend(local.keys().cloned());
        keys.extend(remote.keys().cloned());
        // Include explicit user deletions even when no baseline was ever committed.
        keys.extend(db.list_caldav_tombstones()?);
        let mut count = Counts::default();

        for key in keys {
            let locally_deleted = db.has_caldav_tombstone(&key)?;
            let old = baseline.items.get(&key);
            let l = local.get(&key);
            let r = remote.get(&key);
            let lj = l.map(|x| x.json()).transpose()?;
            let rj = r.map(|x| x.item.json()).transpose()?;
            let oldj = old.map(|x| x.json.as_str());
            let lc = changed(oldj, lj.as_deref());
            let rc = changed(oldj, rj.as_deref());
            match mode {
                SyncMode::LocalToCloud => {
                    if (old.is_some() && l.is_none()) || locally_deleted {
                        if let Some(rr)=r { delete_remote(cfg,&agent,rr)?; count.del_cloud+=1; }
                        db.clear_caldav_tombstone(&key)?;
                    } else if let Some(li)=l {
                        // The local side is authoritative in this mode: recreate a cloud
                        // resource if it disappeared, even when the local item is unchanged.
                        if old.is_none() || r.is_none() || lc { put_remote(cfg,&agent,target,li,r)?; if old.is_some() && r.is_some(){count.up_cloud+=1}else{count.new_cloud+=1}; }
                    }
                }
                SyncMode::CloudToLocal => {
                    if old.is_some() && r.is_none() {
                        if l.is_some(){ delete_local(db,&key)?; count.del_local+=1; }
                        db.clear_caldav_tombstone(&key)?;
                    } else if let Some(rr)=r {
                        // The cloud side is authoritative in this mode: restore a local
                        // item that was removed locally, unless the cloud item is absent.
                        if old.is_none() || l.is_none() || rc { upsert_local(db,&rr.item)?; if old.is_some() && l.is_some(){count.up_local+=1}else{count.new_local+=1}; }
                    } else {
                        db.clear_caldav_tombstone(&key)?;
                    }
                }
                SyncMode::Merge => match (old,l,r) {
                    (None,Some(li),None) => { put_remote(cfg,&agent,target,li,None)?; count.new_cloud+=1; }
                    (None,None,Some(rr)) => {
                        if locally_deleted {
                            // The user explicitly deleted this UID locally, even if the
                            // baseline was lost/reset. Do not resurrect it from the server.
                            delete_remote(cfg,&agent,rr)?;
                            db.clear_caldav_tombstone(&key)?;
                            count.del_cloud+=1;
                        } else {
                            upsert_local(db,&rr.item)?;
                            count.new_local+=1;
                        }
                    }
                    (None,None,None) => { db.clear_caldav_tombstone(&key)?; },
                    (None,Some(li),Some(rr)) => {
                        if li.json()? != rr.item.json()? {
                            // Concurrent first creation with the same UID: preserve local as a new item.
                            let duplicate = new_uid(li); upsert_local(db,&rr.item)?; upsert_local(db,&duplicate)?;
                            put_remote(cfg,&agent,target,&duplicate,None)?; count.conflicts+=1;
                        }
                    }
                    (Some(_),None,None) => { db.clear_caldav_tombstone(&key)?; }
                    (Some(_old_item),Some(li),None) => {
                        if lc {
                            // Remote deletion while local changed: recreate from local copy.
                            put_remote(cfg,&agent,target,li,None)?; count.new_cloud+=1;
                        } else { delete_local(db,&key)?; count.del_local+=1; db.clear_caldav_tombstone(&key)?; }
                    }
                    (Some(_),None,Some(rr)) => {
                        if rc {
                            // A remote edit raced with a local deletion. Preserve it under a new UID.
                            let duplicate=new_uid(&rr.item); upsert_local(db,&duplicate)?;
                            put_remote(cfg,&agent,target,&duplicate,None)?;
                            delete_remote(cfg,&agent,rr)?; db.clear_caldav_tombstone(&key)?; count.conflicts+=1;
                        } else { delete_remote(cfg,&agent,rr)?; db.clear_caldav_tombstone(&key)?; count.del_cloud+=1; }
                    }
                    (Some(_),Some(li),Some(rr)) => {
                        if lc && rc {
                            if li.json()? != rr.item.json()? {
                                // Keep both concurrent versions; cloud version retains original UID.
                                let duplicate=new_uid(li); upsert_local(db,&rr.item)?;
                                upsert_local(db,&duplicate)?; put_remote(cfg,&agent,target,&duplicate,None)?;
                                count.conflicts+=1;
                            }
                        } else if lc { put_remote(cfg,&agent,target,li,Some(rr))?; count.up_cloud+=1; }
                        else if rc { upsert_local(db,&rr.item)?; count.up_local+=1; }
                    }
                }
            }
        }

        // Verify server state before committing the new baseline. If this REPORT fails,
        // leave the previous baseline intact so the next run can safely retry.
        // Verify both component types independently before committing the baseline.
        let verify_xml_events = fetch_calendar_report(cfg, &agent, target, "VEVENT")
            .context("同步后校验 VEVENT 失败；保留旧同步基线")?;
        let verify_xml_todos = fetch_calendar_report(cfg, &agent, target, "VTODO")
            .context("同步后校验 VTODO 失败；保留旧同步基线")?;
        let final_events=db.list_events()?; let final_todos=db.list_todos()?;
        let mut final_local=BTreeMap::new();
        for v in final_events {final_local.insert(format!("E:{}",v.id),Item::Event(v));}
        for v in final_todos {final_local.insert(format!("T:{}",v.id),Item::Todo(v));}
        let mut next=Baseline::default();
        let verify_responses = parse_dav_responses(&verify_xml_events)?
            .into_iter()
            .chain(parse_dav_responses(&verify_xml_todos)?)
            .collect::<Vec<_>>();
        for (index, response) in verify_responses.iter().enumerate() {
            let href = response.href.as_deref().filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("同步后校验时，第 {} 个 DAV response 缺少 href；保留旧同步基线", index + 1))?;
            let Some(raw) = response.calendar_data.as_deref() else {
                bail!("同步后校验资源 {} 缺少 calendar-data；保留旧同步基线", href);
            };
            let (evs,tds)=ical::import_from_ics_with_progress(raw,&calendar_id,|_,_|{})?;
            for v in evs {let k=format!("E:{}",v.id); if let Some(item)=final_local.get(&k){next.items.insert(k,BaseItem{json:item.json()?,href:resolve_href(target,href),etag:response.etag.clone()});}}
            for v in tds {let k=format!("T:{}",v.id); if let Some(item)=final_local.get(&k){next.items.insert(k,BaseItem{json:item.json()?,href:resolve_href(target,href),etag:response.etag.clone()});}}
        }
        db.set_caldav_sync_state(&state_key,&serde_json::to_string(&next)?)?;
        Ok(format!("[{}] {} 完成：上传新增 {}、更新 {}、删除 {}；下载新增 {}、更新 {}、删除 {}；冲突保留 {}。",
            cfg.provider,mode.label(),count.new_cloud,count.up_cloud,count.del_cloud,count.new_local,count.up_local,count.del_local,count.conflicts))
    }

}


#[derive(Debug,Clone,Copy,PartialEq,Eq)] pub enum SyncMode { LocalToCloud, CloudToLocal, Merge }
impl SyncMode { pub fn label(self)->&'static str {match self{Self::LocalToCloud=>"本机→云端",Self::CloudToLocal=>"云端→本机",Self::Merge=>"双向合并同步"}} }
#[derive(Debug,Clone,Serialize,Deserialize,Default)] struct Baseline { items:BTreeMap<String,BaseItem> }
#[derive(Debug,Clone,Serialize,Deserialize)] struct BaseItem { json:String, href:String, etag:Option<String> }
#[derive(Debug,Clone)] struct Remote { item:Item, href:String, etag:Option<String> }
#[derive(Debug,Clone,Serialize,Deserialize)] #[serde(tag="kind",content="value")] enum Item { Event(Event), Todo(Todo) }
impl Item { fn json(&self)->Result<String>{Ok(serde_json::to_string(self)?)} }
#[derive(Default)] struct Counts {new_cloud:usize,up_cloud:usize,del_cloud:usize,new_local:usize,up_local:usize,del_local:usize,conflicts:usize}
fn changed(old:Option<&str>,now:Option<&str>)->bool {match(old,now){(None,Some(_))=>true,(Some(a),Some(b))=>a!=b,(Some(_),None)=>true,_=>false}}
fn new_uid(i:&Item)->Item {match i{Item::Event(v)=>{let mut x=v.clone();x.id=Uuid::new_v4().to_string();Item::Event(x)},Item::Todo(v)=>{let mut x=v.clone();x.id=Uuid::new_v4().to_string();Item::Todo(x)}}}
fn upsert_local(db:&Database,i:&Item)->Result<()> {match i{Item::Event(v)=>db.upsert_event(v),Item::Todo(v)=>db.upsert_todo(v)}}
fn delete_local(db:&Database,k:&str)->Result<()> {if let Some(id)=k.strip_prefix("E:"){db.delete_event_from_sync(id)}else if let Some(id)=k.strip_prefix("T:"){db.delete_todo_from_sync(id)}else{Ok(())}}
fn calendar_query_body(component: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">\n\
 <d:prop><d:getetag/><c:calendar-data/></d:prop>\n\
 <c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"{}\"/></c:comp-filter></c:filter>\n\
</c:calendar-query>",
        component
    )
}

fn fetch_calendar_report(
    cfg: &SyncAccountConfig,
    agent: &ureq::Agent,
    target: &str,
    component: &str,
) -> Result<String> {
    if component != "VEVENT" && component != "VTODO" {
        bail!("不支持的 CalDAV 组件类型：{}", component);
    }
    let body = calendar_query_body(component);
    let response = match SyncEngine::apply_auth(agent.request("REPORT", target), cfg)
        .set("Depth", "1")
        .set("Content-Type", "application/xml; charset=utf-8")
        .send_string(&body)
    {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            bail!("CalDAV {} REPORT 失败：HTTP {} {}", component, code, response.status_text())
        }
        Err(ureq::Error::Transport(error)) => return Err(error.into()),
    };
    if response.status() != 207 && !(200..300).contains(&response.status()) {
        bail!("CalDAV {} REPORT 返回 HTTP {}", component, response.status());
    }
    let xml = response.into_string().context("读取 CalDAV REPORT 失败")?;
    if !xml.contains("multistatus") && !xml.contains("response") {
        bail!("CalDAV {} REPORT 未返回有效 DAV multistatus", component);
    }
    Ok(xml)
}

fn resolve_href(base:&str,href:&str)->String {if href.starts_with("http://")||href.starts_with("https://"){return href.into()} if href.starts_with('/') {if let Some(p)=base.find("://"){let rest=&base[p+3..];let n=rest.find('/').unwrap_or(rest.len());return format!("{}{}",&base[..p+3+n],href)}} format!("{}/{}",base.trim_end_matches('/'),href.trim_start_matches('/'))}
fn put_remote(cfg:&SyncAccountConfig,a:&ureq::Agent,target:&str,item:&Item,current:Option<&Remote>)->Result<()> {
    let (id, calendar_id, body) = match item {
        Item::Event(v) => (v.id.as_str(), v.calendar_id.as_str(), ical::export_to_ics(&[v.clone()], &[])),
        Item::Todo(v) => (v.id.as_str(), v.calendar_id.as_str(), ical::export_to_ics(&[], &[v.clone()])),
    };
    // Keep a deterministic resource name for idempotent retries. The UID is still
    // the logical identity; href is only the server-side resource location.
    let url = current.map(|r| r.href.clone())
        .unwrap_or_else(|| format!("{}/{}.ics", target.trim_end_matches('/'), path_segment(id)));
    let mut req = SyncEngine::apply_auth(a.put(&url), cfg)
        .set("Content-Type", "text/calendar; charset=utf-8");
    if let Some(r) = current {
        let etag = r.etag.as_ref().ok_or_else(|| anyhow::anyhow!(
            "服务器未返回 ETag，拒绝无条件覆盖资源 {}", url
        ))?;
        req = req.set("If-Match", etag);
        return match req.send_string(&body) {
            Ok(resp) if (200..300).contains(&resp.status()) => Ok(()),
            Ok(resp) => bail!("更新 CalDAV 资源失败：PUT {} HTTP {}", url, resp.status()),
            Err(ureq::Error::Status(412, _)) => bail!(
                "云端日程在同步期间又被修改（HTTP 412）：{}。为避免覆盖其他设备的修改，本次未覆盖；请重新点击同步。", url
            ),
            Err(ureq::Error::Status(code, resp)) => bail!("PUT {} HTTP {} {}", url, code, resp.status_text()),
            Err(ureq::Error::Transport(err)) => Err(err.into()),
        };
    }

    // Creation must be conditional. If the server says the resource already
    // exists (412), inspect that exact href instead of blindly retrying without
    // If-None-Match, which could overwrite another calendar object.
    req = req.set("If-None-Match", "*");
    match req.send_string(&body) {
        Ok(resp) if (200..300).contains(&resp.status()) => Ok(()),
        Ok(resp) => bail!("创建 CalDAV 资源失败：PUT {} HTTP {}", url, resp.status()),
        Err(ureq::Error::Status(412, _)) => {
            let Some((remote_item, remote_etag)) = fetch_resource_at_url(cfg, a, &url, calendar_id)? else {
                bail!("服务器对新建资源返回 HTTP 412，但随后 GET {} 却返回 404。服务器的条件 PUT 行为异常；为避免无条件覆盖，已停止本次同步。", url);
            };
            let remote_uid = match &remote_item { Item::Event(v) => v.id.as_str(), Item::Todo(v) => v.id.as_str() };
            if remote_uid != id {
                bail!("HTTP 412：目标资源 URL 已被另一个 UID 占用（本地 UID={}，云端 UID={}，URL={}）。为保护云端数据，没有覆盖该资源。", id, remote_uid, url);
            }
            if remote_item.json()? == item.json()? {
                // A previous PUT may have reached the server even if the client
                // did not receive/commit its response. Treat identical content as
                // an idempotent success.
                Ok(())
            } else {
                let etag = remote_etag.ok_or_else(|| anyhow::anyhow!(
                    "HTTP 412 后发现同 UID 云端资源，但服务器未提供 ETag；拒绝无条件更新 {}", url
                ))?;
                let retry = SyncEngine::apply_auth(a.put(&url), cfg)
                    .set("Content-Type", "text/calendar; charset=utf-8")
                    .set("If-Match", &etag);
                match retry.send_string(&body) {
                    Ok(resp) if (200..300).contains(&resp.status()) => Ok(()),
                    Ok(resp) => bail!("HTTP 412 恢复更新失败：PUT {} HTTP {}", url, resp.status()),
                    Err(ureq::Error::Status(412, _)) => bail!("云端资源再次发生并发修改（HTTP 412）：{}。本次未覆盖，请重新同步。", url),
                    Err(ureq::Error::Status(code, resp)) => bail!("HTTP 412 恢复更新失败：PUT {} HTTP {} {}", url, code, resp.status_text()),
                    Err(ureq::Error::Transport(err)) => Err(err.into()),
                }
            }
        }
        Err(ureq::Error::Status(code, resp)) => bail!("PUT {} HTTP {} {}", url, code, resp.status_text()),
        Err(ureq::Error::Transport(err)) => Err(err.into()),
    }
}

/// Read a resource after a conditional-create 412. This is deliberately an
/// exact-URL GET, not a broad search: it lets us distinguish an idempotent retry
/// from a real href/UID collision without weakening write preconditions.
fn fetch_resource_at_url(
    cfg: &SyncAccountConfig,
    agent: &ureq::Agent,
    url: &str,
    calendar_id: &str,
) -> Result<Option<(Item, Option<String>)>> {
    let response = match SyncEngine::apply_auth(agent.get(url), cfg).call() {
        Ok(resp) => resp,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(ureq::Error::Status(code, resp)) => bail!(
            "处理 PUT HTTP 412 时无法读取冲突资源：GET {} HTTP {} {}", url, code, resp.status_text()
        ),
        Err(ureq::Error::Transport(err)) => return Err(err.into()),
    };
    let etag = response.header("ETag").map(str::to_string);
    let ics = response.into_string().context("读取 HTTP 412 冲突资源内容失败")?;
    let (events, todos) = ical::import_from_ics_with_progress(&ics, calendar_id, |_, _| {})
        .context("HTTP 412 冲突资源不是有效的 iCalendar 数据")?;
    if events.len() + todos.len() != 1 {
        bail!("HTTP 412 冲突资源 {} 包含 {} 个 VEVENT/VTODO，无法安全判断 UID，已停止覆盖", url, events.len() + todos.len());
    }
    let item = if let Some(event) = events.into_iter().next() {
        Item::Event(event)
    } else {
        Item::Todo(todos.into_iter().next().expect("exactly one parsed component"))
    };
    Ok(Some((item, etag)))
}
fn delete_remote(cfg:&SyncAccountConfig,a:&ureq::Agent,r:&Remote)->Result<()> {let e=r.etag.as_ref().ok_or_else(||anyhow::anyhow!("服务器未返回 ETag，拒绝无条件删除资源 {}",r.href))?;let mut q=SyncEngine::apply_auth(a.request("DELETE",&r.href),cfg).set("If-Match",e); match q.call(){Ok(x) if (200..300).contains(&x.status())||x.status()==404=>Ok(()),Err(ureq::Error::Status(404,_))=>Ok(()),Err(ureq::Error::Status(c,x))=>bail!("DELETE {} HTTP {} {}",r.href,c,x.status_text()),Err(ureq::Error::Transport(e))=>Err(e.into()),Ok(x)=>bail!("DELETE {} HTTP {}",r.href,x.status())}}
#[derive(Debug, Default)]
struct DavResponse {
    href: Option<String>,
    etag: Option<String>,
    calendar_data: Option<String>,
}

/// Parse DAV multistatus by XML local-name, not by literal tag spelling.
/// This accepts arbitrary namespace prefixes, default namespaces, attributes,
/// whitespace/newlines, and both normal and self-closing XML elements.
fn parse_dav_responses(xml: &str) -> Result<Vec<DavResponse>> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut responses = Vec::new();
    let mut current: Option<DavResponse> = None;
    let mut capture: Option<String> = None;
    let mut captured = String::new();

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "response" {
                    if current.is_some() {
                        bail!("CalDAV XML 中出现嵌套 DAV response，无法安全解析");
                    }
                    current = Some(DavResponse::default());
                } else if current.is_some() && matches!(name.as_str(), "href" | "getetag" | "calendar-data") {
                    capture = Some(name);
                    captured.clear();
                }
            }
            Ok(XmlEvent::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "response" {
                    responses.push(DavResponse::default());
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if capture.is_some() {
                    captured.push_str(&e.decode().context("解码 CalDAV XML 文本失败")?);
                }
            }
            Ok(XmlEvent::CData(e)) => {
                if capture.is_some() {
                    captured.push_str(&e.decode().context("解码 CalDAV XML CDATA 失败")?);
                }
            }
            Ok(XmlEvent::End(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if let Some(field) = capture.as_deref() {
                    if field == name {
                        let value = quick_xml::escape::unescape(captured.trim())
                            .context("解码 DAV XML 实体失败")?
                            .into_owned();
                        if let Some(response) = current.as_mut() {
                            match field {
                                "href" => response.href = Some(value),
                                "getetag" => response.etag = Some(value),
                                "calendar-data" => response.calendar_data = Some(value),
                                _ => {}
                            }
                        }
                        capture = None;
                        captured.clear();
                    }
                }
                if name == "response" {
                    if let Some(response) = current.take() {
                        responses.push(response);
                    }
                }
            }
            Ok(XmlEvent::Eof) => break,
            Ok(_) => {}
            Err(error) => bail!("CalDAV 返回的 XML 格式无效：{}", error),
        }
    }
    if current.is_some() {
        bail!("CalDAV XML 中 DAV response 标签未闭合");
    }
    Ok(responses)
}
fn xml_unescape(s:&str)->String{s.replace("&#13;","\r").replace("&#10;","\n").replace("&#xD;","\r").replace("&#xA;","\n").replace("&lt;","<").replace("&gt;",">").replace("&quot;", "\"").replace("&apos;","'").replace("&amp;","&")}
fn path_segment(s:&str)->String{let mut o=String::new();for b in s.bytes(){if b.is_ascii_alphanumeric()||b"-._~".contains(&b){o.push(b as char)}else{o.push_str(&format!("%{:02X}",b))}}o}

#[cfg(test)]
mod caldav_query_tests {
    use super::{calendar_query_body, parse_dav_responses};

    #[test]
    fn calendar_query_filters_exactly_one_component_type() {
        for component in ["VEVENT", "VTODO"] {
            let body = calendar_query_body(component);
            assert!(body.contains(&format!("name=\"{}\"", component)));
            assert_eq!(body.matches("<c:comp-filter").count(), 2);
            assert!(!body.contains("name=\"VEVENT\"/><c:comp-filter name=\"VTODO"));
        }
    }

    #[test]
    fn parses_dav_response_with_different_namespace_prefixes() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:response><d:href>/cal/a.ics</d:href><d:propstat><d:prop>
    <d:getetag>"tag-1"</d:getetag><c:calendar-data>BEGIN:VCALENDAR&#13;&#10;END:VCALENDAR</c:calendar-data>
  </d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let parsed = parse_dav_responses(xml).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].href.as_deref(), Some("/cal/a.ics"));
        assert_eq!(parsed[0].etag.as_deref(), Some("\"tag-1\""));
        assert!(parsed[0].calendar_data.as_deref().unwrap().contains("BEGIN:VCALENDAR\r\n"));
    }

    #[test]
    fn parses_default_dav_namespace_and_empty_response() {
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/cal/b.ics</href></response><response/></multistatus>"#;
        let parsed = parse_dav_responses(xml).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].href.as_deref(), Some("/cal/b.ics"));
        assert!(parsed[1].href.is_none());
    }
}

/// 零依赖标准 Base64 编码实现（用于 HTTP Basic Auth）
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

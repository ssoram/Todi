// ════════ 자동 동기화 ════════
//
// 같은 "동기화 코드"를 넣은 기기끼리 고른 카테고리의 항목을 맞춘다. 방법은 두 가지이고 함께 써도 된다.
//
// - 같은 와이파이: UDP로 "나 여기 있어" 신호(beacon)를 뿌리고, 같은 코드의 기기를 찾으면 TCP로 직접 주고받는다.
//   인터넷으로 나가지 않는다. 켜기 전에는 포트를 열지 않는다.
//   신호는 네트워크 어댑터마다 따로 뿌린다 (유선·무선·가상 어댑터가 섞인 PC에서도 찾을 수 있게).
//   신호를 받으면 보낸 쪽에 바로 답장을 보내서, 한쪽 방화벽이 들어오는 연결을 막아도
//   그쪽에서 먼저 연결해 맞출 수 있게 한다. 신호가 닿지 않는 네트워크면 상대 주소를 직접 넣을 수도 있다.
// - 폴더: 두 기기가 함께 보는 폴더(OneDrive, 구글 드라이브, USB 등)에 기기마다 자기 파일
//   (todi-sync-{기기ID}.todi)을 쓰고, 다른 기기의 파일을 읽어서 합친다. 기기마다 파일이 달라서 서로 덮어쓰지 않는다.
//
// 주고받는 내용은 모두 코드에서 만든 키로 암호화(ChaCha20-Poly1305)한다.
// 코드는 무작위 80비트라서, 코드를 모르면 파일이나 네트워크 내용을 읽거나 위조할 수 없다.
//
// 카테고리: 각 기기에서 체크한 카테고리를 "보낸다". 상대가 보낸 카테고리는 받는 쪽에서 따로 고르지 않아도
// 자동으로 함께 맞춘다. 받는 쪽에서 체크를 끈 카테고리(sync_off)만 받지 않는다.
//
// 합치는 규칙 (merge):
// - 항목은 uid로 같은 항목인지 알아보고, updated(마지막 수정 시각)가 더 최근인 쪽을 남긴다.
// - 완전히 지운 항목은 삭제 기록(deleted)을 주고받아서 양쪽에서 지운다.
// - 동기화하지 않는 카테고리로 옮겨진 항목은 내용 없이 "옮겨짐"(moved)만 알려서 상대도 카테고리를 바꾼다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::crypto::{new_code, normalize_code, open, random_bytes, seal, sha256, to_hex};
use crate::{emit_refresh, lock, memo_hash, AppData, Memo, Store};

/// 같은 와이파이에서 서로를 찾는 UDP 포트
const BEACON_PORT: u16 = 47321;
/// 동기화 주기 (beacon 전송 / 폴더 확인)
const TICK: Duration = Duration::from_secs(3);
/// 이 시간 동안 신호가 없으면 "연결 끊김"으로 표시
const ONLINE_FOR: Duration = Duration::from_secs(15);
const MAX_FRAME: usize = 64 * 1024 * 1024;

// ──────── 주고받는 내용 ────────

#[derive(Serialize, Deserialize, Default)]
struct SyncDoc {
    device: String,
    #[serde(default)]
    name: String,
    /// 동기화하는 카테고리의 항목 (휴지통 포함 — 휴지통으로 보낸 것도 맞춰야 하므로)
    #[serde(default)]
    memos: Vec<Memo>,
    /// 동기화하지 않는 카테고리에 있는 항목: 내용 없이 uid와 카테고리만
    #[serde(default)]
    moved: Vec<Moved>,
    #[serde(default)]
    deleted: HashMap<String, i64>,
}

#[derive(Serialize, Deserialize)]
struct Moved {
    uid: String,
    updated: i64,
    category: String,
}

/// 같은 와이파이에서 뿌리는 신호. 내용은 없고, 같은 코드인지 알아볼 표식(g)만 있다.
#[derive(Serialize, Deserialize)]
struct Beacon {
    t: String,
    d: String,
    p: u16,
    g: String,
    r: u64,
    /// 0 = 뿌린 신호, 1 = 받은 신호에 대한 답장
    #[serde(default)]
    k: u8,
}

// ──────── 실행 중 상태 ────────

struct Peer {
    name: String,
    ip: IpAddr,
    port: u16,
    /// 마지막으로 맞춘 뒤 상대의 변경 번호 / 내가 보낸 변경 번호
    seen_rev: Option<u64>,
    sent_rev: Option<u64>,
    last_sync: String,
    last_seen: Instant,
}

pub(crate) struct SyncRt {
    /// 데이터가 바뀔 때마다 올라가는 번호. 상대가 이 번호가 바뀐 걸 보고 다시 맞추러 온다.
    rev: AtomicU64,
    peers: Mutex<HashMap<String, Peer>>,
    busy: Mutex<HashSet<String>>,
    tcp_port: AtomicU64,
    lan_started: AtomicBool,
    folder_seen: Mutex<HashMap<PathBuf, SystemTime>>,
    folder_written: AtomicU64,
    folder_last: Mutex<String>,
    error: Mutex<String>,
    /// 신호를 주고받는 UDP 소켓 (답장이 방화벽을 통과하도록 보내고 받는 포트를 같게)
    sock: Mutex<Option<Arc<UdpSocket>>>,
    /// 와이파이 동기화를 켠 시각 (한동안 상대를 못 찾으면 도움말을 보여 주려고)
    lan_since: Mutex<Option<Instant>>,
    /// 진단용: 코드가 다른 Todi의 신호를 마지막으로 받은 시각과 주소
    other_code: Mutex<Option<(Instant, IpAddr)>>,
    /// 진단용: 주소별로 Todi 신호를 마지막으로 받은 시각 (직접 넣은 주소가 응답하는지 보여 주려고)
    heard: Mutex<HashMap<IpAddr, Instant>>,
}

impl SyncRt {
    pub(crate) fn new() -> Self {
        let b: [u8; 8] = random_bytes();
        Self {
            // 껐다 켜면 번호가 달라져서 상대와 다시 한 번 맞추게 된다
            rev: AtomicU64::new(u64::from_le_bytes(b) >> 1),
            peers: Mutex::new(HashMap::new()),
            busy: Mutex::new(HashSet::new()),
            tcp_port: AtomicU64::new(0),
            lan_started: AtomicBool::new(false),
            folder_seen: Mutex::new(HashMap::new()),
            folder_written: AtomicU64::new(u64::MAX),
            folder_last: Mutex::new(String::new()),
            error: Mutex::new(String::new()),
            sock: Mutex::new(None),
            lan_since: Mutex::new(None),
            other_code: Mutex::new(None),
            heard: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn bump(&self) {
        self.rev.fetch_add(1, Ordering::SeqCst);
    }

    fn set_error(&self, e: impl Into<String>) {
        *self.error.lock().unwrap() = e.into();
    }
}

/// 설정에서 동기화에 필요한 값만 꺼내 둔 것
struct Cfg {
    key: Option<[u8; 32]>,
    device: String,
    cats: Vec<String>,
    lan: bool,
    peer: String,
    folder: String,
    folder_on: bool,
}

fn cfg(app: &AppHandle) -> Cfg {
    let store = app.state::<Store>();
    let s = store.settings.lock().unwrap();
    Cfg {
        key: (!s.sync_code.is_empty()).then(|| key_from_code(&s.sync_code)),
        device: s.device_id.clone(),
        cats: s.sync_cats.clone(),
        lan: s.sync_lan,
        peer: s.sync_peer.clone(),
        folder: s.sync_folder.clone(),
        folder_on: s.sync_folder_on,
    }
}

fn now_str() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "다른 컴퓨터".into())
}

// ──────── 코드 / 암호화 ────────

fn key_from_code(code: &str) -> [u8; 32] {
    sha256(&[b"todi-sync-v1:", code.as_bytes()])
}

/// 같은 코드인지 알아보는 표식 (키 자체는 드러나지 않음)
fn group_tag(key: &[u8; 32]) -> String {
    to_hex(&sha256(&[key, b"group-tag"])[..8])
}

// ──────── 만들기 / 합치기 ────────

fn build_doc(app: &AppHandle, c: &Cfg) -> SyncDoc {
    let store = app.state::<Store>();
    let d = store.data.lock().unwrap();
    let (mine, others): (Vec<&Memo>, Vec<&Memo>) = d.memos.iter().partition(|m| c.cats.contains(&m.category));
    SyncDoc {
        device: c.device.clone(),
        name: device_name(),
        memos: mine.into_iter().cloned().collect(),
        moved: others.into_iter()
            .map(|m| Moved { uid: m.uid.clone(), updated: m.updated, category: m.category.clone() })
            .collect(),
        deleted: d.deleted.clone(),
    }
}

/// 받은 내용을 합친다. 바뀐 게 있으면 저장하고 모든 창을 새로 그린다.
fn merge(app: &AppHandle, doc: SyncDoc) {
    let store = app.state::<Store>();
    let (changed, cats_changed) = {
        let mut d = store.data.lock().unwrap();
        let mut s = store.settings.lock().unwrap();
        // 잠겨 있는 카테고리는 지금 내용을 넣을 수 없으므로 건너뛰고, 잠금을 풀면 다시 맞춘다
        let hidden = lock::hidden_filter(&s, store.lock_key.lock().unwrap().is_some());
        let mut sync_cats = std::mem::take(&mut s.sync_cats);
        let r = merge_into(&mut d, &mut sync_cats, &s.sync_off, doc, &hidden);
        s.sync_cats = sync_cats;
        r
    };
    if cats_changed {
        store.save_settings();
    }
    if changed {
        emit_refresh(app, &store);
    }
}

/// 합치기 본체. (데이터가 바뀌었는지, 동기화 카테고리 목록이 바뀌었는지)
fn merge_into(d: &mut AppData, sync_cats: &mut Vec<String>, off: &[String], doc: SyncDoc, hidden: &dyn Fn(&str) -> bool) -> (bool, bool) {
    let mut changed = false;
    let mut cats_changed = false;
    {

        // 1) 지운 항목
        for (uid, t) in doc.deleted {
            let e = d.deleted.entry(uid.clone()).or_insert(0);
            if t > *e {
                *e = t;
                changed = true;
            }
            if let Some(i) = d.memos.iter().position(|m| m.uid == uid && m.updated <= t) {
                d.memos.remove(i);
                changed = true;
            }
        }

        // 2) 상대가 보낸 카테고리는 (이쪽에서 체크를 끈 게 아니면) 이쪽도 함께 맞춘다
        for m in &doc.memos {
            if !sync_cats.contains(&m.category) && !off.contains(&m.category) && !hidden(&m.category) {
                sync_cats.push(m.category.clone());
                cats_changed = true;
            }
        }

        // 3) 항목: 더 최근 것을 남긴다
        let mut next = crate::max_id(d);
        for m in doc.memos {
            if m.uid.is_empty() || hidden(&m.category) || d.deleted.get(&m.uid).is_some_and(|&t| t >= m.updated) {
                continue;
            }
            if let Some(l) = d.memos.iter_mut().find(|l| l.uid == m.uid) {
                if m.updated > l.updated {
                    let was_synced = sync_cats.contains(&l.category);
                    *l = Memo { id: l.id, ..m };
                    l.hash = memo_hash(l);
                    // 상대가 카테고리 이름을 바꿨으면 새 이름도 계속 동기화
                    if was_synced && !sync_cats.contains(&l.category) {
                        sync_cats.push(l.category.clone());
                        cats_changed = true;
                    }
                    changed = true;
                }
            } else if sync_cats.contains(&m.category) {
                next += 1;
                let mut m = Memo { id: next, ..m };
                m.hash = memo_hash(&m);
                d.memos.push(m);
                changed = true;
            }
        }

        // 4) 동기화 안 하는 카테고리로 옮겨진 항목
        for mv in doc.moved {
            if let Some(l) = d.memos.iter_mut().find(|l| l.uid == mv.uid) {
                if mv.updated > l.updated && l.category != mv.category && !hidden(&mv.category) {
                    l.category = mv.category;
                    l.updated = mv.updated;
                    l.hash = memo_hash(l);
                    changed = true;
                }
            }
        }

        // 5) 처음 보는 카테고리는 목록에 추가
        let missing: Vec<String> = d.memos.iter().map(|m| m.category.clone())
            .filter(|c| !d.categories.contains(c)).collect::<HashSet<_>>().into_iter().collect();
        for c in missing {
            d.categories.push(c);
            changed = true;
        }
    }
    (changed, cats_changed)
}

// ──────── 폴더 동기화 ────────

fn folder_tick(app: &AppHandle, rt: &SyncRt, c: &Cfg, key: &[u8; 32]) {
    let dir = PathBuf::from(&c.folder);
    if c.folder.is_empty() || !dir.is_dir() {
        rt.set_error("동기화 폴더를 찾을 수 없어요. 폴더를 다시 골라 주세요.");
        return;
    }
    let own_name = format!("todi-sync-{}.todi", c.device);
    let own = dir.join(&own_name);

    // 내 파일 쓰기 (바뀐 게 있을 때만)
    let rev = rt.rev.load(Ordering::SeqCst);
    if rt.folder_written.load(Ordering::SeqCst) != rev || !own.exists() {
        let doc = build_doc(app, c);
        let data = seal(key, &serde_json::to_vec(&doc).unwrap_or_default());
        let tmp = dir.join(format!(".{own_name}.tmp"));
        match fs::write(&tmp, &data).and_then(|_| fs::rename(&tmp, &own)) {
            Ok(_) => {
                rt.folder_written.store(rev, Ordering::SeqCst);
                *rt.folder_last.lock().unwrap() = now_str();
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                rt.set_error(format!("동기화 폴더에 저장하지 못했어요: {e}"));
                return;
            }
        }
    }

    // 다른 기기의 파일 읽기 (바뀐 파일만)
    let Ok(entries) = fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("todi-sync-") || !name.ends_with(".todi") || name == own_name {
            continue;
        }
        let path = e.path();
        let Ok(mtime) = e.metadata().and_then(|m| m.modified()) else { continue };
        if rt.folder_seen.lock().unwrap().get(&path) == Some(&mtime) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        rt.folder_seen.lock().unwrap().insert(path, mtime);
        // 코드가 다른 기기의 파일이면 열리지 않으므로 그냥 넘어간다
        let Some(plain) = open(key, &bytes) else { continue };
        let Ok(doc) = serde_json::from_slice::<SyncDoc>(&plain) else { continue };
        if doc.device == c.device {
            continue;
        }
        merge(app, doc);
        *rt.folder_last.lock().unwrap() = now_str();
    }
}

// ──────── 같은 와이파이 동기화 ────────

fn write_frame(s: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    s.write_all(&(data.len() as u32).to_be_bytes())?;
    s.write_all(data)?;
    s.flush()
}

fn read_frame(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "too large"));
    }
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf)?;
    Ok(buf)
}

/// 처음 와이파이 동기화를 켤 때 한 번만 포트를 연다
fn ensure_lan(app: &AppHandle, rt: &SyncRt) {
    if rt.lan_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let listener = match TcpListener::bind(("0.0.0.0", 0)) {
        Ok(l) => l,
        Err(e) => {
            rt.set_error(format!("와이파이 동기화를 시작하지 못했어요: {e}"));
            rt.lan_started.store(false, Ordering::SeqCst);
            return;
        }
    };
    rt.tcp_port.store(listener.local_addr().map(|a| a.port()).unwrap_or(0) as u64, Ordering::SeqCst);
    let a = app.clone();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let a = a.clone();
            std::thread::spawn(move || serve(&a, s));
        }
    });

    match UdpSocket::bind(("0.0.0.0", BEACON_PORT)) {
        Ok(sock) => {
            let _ = sock.set_broadcast(true);
            let sock = Arc::new(sock);
            *rt.sock.lock().unwrap() = Some(sock.clone());
            let a = app.clone();
            std::thread::spawn(move || listen_beacons(&a, sock));
        }
        Err(e) => rt.set_error(format!("다른 기기를 찾는 포트({BEACON_PORT})를 열지 못했어요: {e}")),
    }
}

/// 이 컴퓨터의 IPv4 주소와 그 네트워크의 브로드캐스트 주소 (가상 어댑터 포함, 루프백·자동 할당 주소 제외)
fn local_v4() -> Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> {
    let Ok(list) = if_addrs::get_if_addrs() else { return vec![] };
    list.into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if !a.ip.is_loopback() && !a.ip.is_link_local() => {
                let b = a.broadcast.unwrap_or_else(|| {
                    std::net::Ipv4Addr::from(u32::from(a.ip) | !u32::from(a.netmask))
                });
                Some((a.ip, b))
            }
            _ => None,
        })
        .collect()
}

/// 직접 넣은 상대 주소들 (쉼표·띄어쓰기로 여러 개)
fn manual_peers(s: &str) -> Vec<IpAddr> {
    s.split([',', ' ', ';']).filter_map(|x| x.trim().parse().ok()).collect()
}

fn beacon_bytes(rt: &SyncRt, c: &Cfg, key: &[u8; 32], k: u8) -> Option<Vec<u8>> {
    let port = rt.tcp_port.load(Ordering::SeqCst) as u16;
    if port == 0 {
        return None;
    }
    let b = Beacon { t: "todi".into(), d: c.device.clone(), p: port, g: group_tag(key), r: rt.rev.load(Ordering::SeqCst), k };
    serde_json::to_vec(&b).ok()
}

fn send_beacon(rt: &SyncRt, c: &Cfg, key: &[u8; 32]) {
    let Some(json) = beacon_bytes(rt, c, key, 0) else { return };
    let Some(sock) = rt.sock.lock().unwrap().clone() else { return };
    // 어댑터마다 그 네트워크의 브로드캐스트 주소로 (255.255.255.255는 Windows에서 어댑터 하나로만 나감)
    let mut targets: Vec<IpAddr> = local_v4().into_iter().map(|(_, b)| IpAddr::V4(b)).collect();
    targets.push(IpAddr::V4(std::net::Ipv4Addr::BROADCAST));
    targets.extend(manual_peers(&c.peer));
    targets.dedup();
    for t in targets {
        let _ = sock.send_to(&json, SocketAddr::new(t, BEACON_PORT));
    }
}

fn listen_beacons(app: &AppHandle, sock: Arc<UdpSocket>) {
    let rt = app.state::<SyncRt>();
    let mut buf = [0u8; 2048];
    loop {
        let (n, src) = match sock.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => {
                // Windows는 상대가 없을 때 받은 "도달 불가" 응답을 오류로 돌려준다 → 잠깐 쉬고 계속
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) else { continue };
        let c = cfg(app);
        let Some(key) = c.key else { continue };
        if !c.lan || b.t != "todi" || b.d == c.device {
            continue;
        }
        rt.heard.lock().unwrap().insert(src.ip(), Instant::now());
        if b.g != group_tag(&key) {
            // 같은 네트워크에 Todi는 있는데 코드가 다름 → 화면에 알려 준다
            *rt.other_code.lock().unwrap() = Some((Instant::now(), src.ip()));
            continue;
        }
        // 뿌린 신호를 받았으면 보낸 쪽에 바로 답장 → 그쪽 방화벽이 들어오는 신호를 막아도 답장은 통과한다
        if b.k == 0 {
            if let Some(json) = beacon_bytes(&rt, &c, &key, 1) {
                let _ = sock.send_to(&json, src);
            }
        }
        let my_rev = rt.rev.load(Ordering::SeqCst);
        let need = {
            let mut peers = rt.peers.lock().unwrap();
            let p = peers.entry(b.d.clone()).or_insert_with(|| Peer {
                name: String::new(), ip: src.ip(), port: b.p, seen_rev: None, sent_rev: None,
                last_sync: String::new(), last_seen: Instant::now(),
            });
            p.ip = src.ip();
            p.port = b.p;
            p.last_seen = Instant::now();
            p.seen_rev != Some(b.r) || p.sent_rev != Some(my_rev)
        };
        if need && rt.busy.lock().unwrap().insert(b.d.clone()) {
            let a = app.clone();
            let addr = SocketAddr::new(src.ip(), b.p);
            std::thread::spawn(move || {
                let rt = a.state::<SyncRt>();
                if let Err(e) = exchange(&a, addr, &b.d, b.r) {
                    // 상대 방화벽이 들어오는 연결을 막으면 실패하지만, 그때는 상대 쪽에서 먼저 연결해 온다.
                    // 한 번도 맞춘 적이 없을 때만 알린다.
                    let never = rt.peers.lock().unwrap().get(&b.d).is_none_or(|p| p.last_sync.is_empty());
                    if never {
                        rt.set_error(format!("다른 컴퓨터를 찾았지만 연결하지 못했어요. 그 컴퓨터의 Windows 방화벽에서 Todi를 허용했는지 확인해 주세요. ({e})"));
                    }
                }
                rt.busy.lock().unwrap().remove(&b.d);
            });
        }
    }
}

/// 내가 먼저 연결: 내 내용을 보내고 상대 내용을 받아서 합친다
fn exchange(app: &AppHandle, addr: SocketAddr, peer: &str, peer_rev: u64) -> Result<(), String> {
    let c = cfg(app);
    let key = c.key.ok_or("코드 없음")?;
    let rt = app.state::<SyncRt>();
    let my_rev = rt.rev.load(Ordering::SeqCst);
    let mine = build_doc(app, &c);

    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(20)));
    write_frame(&mut s, &seal(&key, &serde_json::to_vec(&mine).unwrap_or_default())).map_err(|e| e.to_string())?;
    let reply = read_frame(&mut s).map_err(|e| e.to_string())?;
    let plain = open(&key, &reply).ok_or("코드가 맞지 않아요")?;
    let doc: SyncDoc = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;

    if let Some(p) = rt.peers.lock().unwrap().get_mut(peer) {
        p.name = doc.name.clone();
        p.seen_rev = Some(peer_rev);
        p.sent_rev = Some(my_rev);
        p.last_sync = now_str();
    }
    rt.set_error("");
    merge(app, doc);
    Ok(())
}

/// 상대가 연결해 왔을 때
fn serve(app: &AppHandle, mut s: TcpStream) {
    let c = cfg(app);
    let Some(key) = c.key else { return };
    if !c.lan {
        return;
    }
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(20)));
    let Ok(frame) = read_frame(&mut s) else { return };
    // 코드가 다르면 열리지 않음 → 아무 것도 보내지 않고 끊는다
    let Some(plain) = open(&key, &frame) else { return };
    let Ok(doc) = serde_json::from_slice::<SyncDoc>(&plain) else { return };
    let rt = app.state::<SyncRt>();
    let my_rev = rt.rev.load(Ordering::SeqCst);
    let mine = build_doc(app, &c);
    if write_frame(&mut s, &seal(&key, &serde_json::to_vec(&mine).unwrap_or_default())).is_err() {
        return;
    }
    if let Ok(addr) = s.peer_addr() {
        let mut peers = rt.peers.lock().unwrap();
        let p = peers.entry(doc.device.clone()).or_insert_with(|| Peer {
            name: String::new(), ip: addr.ip(), port: 0, seen_rev: None, sent_rev: None,
            last_sync: String::new(), last_seen: Instant::now(),
        });
        p.name = doc.name.clone();
        p.sent_rev = Some(my_rev);
        p.last_sync = now_str();
        p.last_seen = Instant::now();
    }
    rt.set_error("");
    merge(app, doc);
}

// ──────── 주기 작업 ────────

pub(crate) fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        let rt = app.state::<SyncRt>();
        let c = cfg(&app);
        if let Some(key) = c.key {
            if c.lan {
                rt.lan_since.lock().unwrap().get_or_insert_with(Instant::now);
                ensure_lan(&app, &rt);
                send_beacon(&rt, &c, &key);
            }
            if c.folder_on {
                folder_tick(&app, &rt, &c, &key);
            }
        }
        std::thread::sleep(TICK);
    });
}

// ──────── 설정 화면 명령 ────────

#[derive(Serialize)]
pub(crate) struct PeerInfo {
    name: String,
    last_sync: String,
    online: bool,
}

#[derive(Serialize)]
pub(crate) struct SyncStatus {
    code: String,
    cats: Vec<String>,
    lan: bool,
    folder: String,
    folder_on: bool,
    peers: Vec<PeerInfo>,
    folder_last: String,
    error: String,
    /// 이 컴퓨터의 주소 (상대 주소를 직접 넣을 때 알려 주는 용도)
    my_ips: Vec<String>,
    peer: String,
    /// 와이파이 동기화를 켠 뒤 지난 초 (상대를 못 찾을 때 도움말을 띄우는 기준)
    lan_secs: u64,
    /// 최근 30초 안에 코드가 다른 Todi의 신호를 받았으면 그 주소
    other_code: String,
    /// 직접 넣은 주소마다 (주소, 최근에 신호를 받았는지)
    manual: Vec<(String, bool)>,
}

#[tauri::command]
pub(crate) fn get_sync_status(app: AppHandle) -> SyncStatus {
    let rt = app.state::<SyncRt>();
    let s = app.state::<Store>().settings.lock().unwrap().clone();
    let peers = rt.peers.lock().unwrap().values()
        .filter(|p| !p.last_sync.is_empty())
        .map(|p| PeerInfo {
            name: if p.name.is_empty() { "다른 컴퓨터".into() } else { p.name.clone() },
            last_sync: p.last_sync.clone(),
            online: p.last_seen.elapsed() < ONLINE_FOR,
        })
        .collect();
    let error = rt.error.lock().unwrap().clone();
    let folder_last = rt.folder_last.lock().unwrap().clone();
    let other_code = rt.other_code.lock().unwrap()
        .filter(|(t, _)| t.elapsed() < Duration::from_secs(30))
        .map(|(_, ip)| ip.to_string()).unwrap_or_default();
    let manual = {
        let heard = rt.heard.lock().unwrap();
        manual_peers(&s.sync_peer).into_iter()
            .map(|ip| (ip.to_string(), heard.get(&ip).is_some_and(|t| t.elapsed() < ONLINE_FOR)))
            .collect()
    };
    let lan_secs = rt.lan_since.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    SyncStatus {
        code: s.sync_code,
        cats: s.sync_cats,
        lan: s.sync_lan,
        folder: s.sync_folder,
        folder_on: s.sync_folder_on,
        peers,
        folder_last,
        error,
        my_ips: local_v4().into_iter().map(|(ip, _)| ip.to_string()).collect(),
        peer: s.sync_peer,
        lan_secs,
        other_code,
        manual,
    }
}

/// 잠금을 푼 뒤 등: 상대/폴더와 처음부터 다시 맞추게 한다
pub(crate) fn resync(app: &AppHandle) {
    if let Some(rt) = app.try_state::<SyncRt>() {
        rt.peers.lock().unwrap().clear();
        rt.folder_seen.lock().unwrap().clear();
        rt.bump();
    }
}

/// 설정을 바꾼 뒤: 저장하고, 상대/폴더와 처음부터 다시 맞추게 한다
fn changed(app: &AppHandle, reset: bool) {
    let store = app.state::<Store>();
    store.save_settings();
    let rt = app.state::<SyncRt>();
    if reset {
        rt.peers.lock().unwrap().clear();
        rt.folder_seen.lock().unwrap().clear();
        *rt.folder_last.lock().unwrap() = String::new();
    }
    rt.set_error("");
    rt.bump();
}

#[tauri::command]
pub(crate) fn sync_new_code(app: AppHandle) -> String {
    let code = new_code();
    app.state::<Store>().settings.lock().unwrap().sync_code = code.clone();
    changed(&app, true);
    code
}

#[tauri::command]
pub(crate) fn sync_set_code(app: AppHandle, code: String) -> Result<String, String> {
    let code = normalize_code(&code).ok_or("코드 형식이 맞지 않아요. 다른 컴퓨터에 표시된 16자리 코드를 그대로 넣어 주세요.")?;
    app.state::<Store>().settings.lock().unwrap().sync_code = code.clone();
    changed(&app, true);
    Ok(code)
}

/// 동기화 끊기: 코드와 방법 설정을 지운다 (항목은 그대로)
#[tauri::command]
pub(crate) fn sync_clear(app: AppHandle) {
    {
        let store = app.state::<Store>();
        let mut s = store.settings.lock().unwrap();
        s.sync_code.clear();
        s.sync_lan = false;
        s.sync_folder_on = false;
    }
    changed(&app, true);
}

#[tauri::command]
pub(crate) fn sync_set_cats(app: AppHandle, cats: Vec<String>) {
    {
        let store = app.state::<Store>();
        let mut s = store.settings.lock().unwrap();
        // 체크를 끈 카테고리는 상대가 보내도 받지 않도록 기억해 둔다
        let unchecked: Vec<String> = s.sync_cats.iter().filter(|c| !cats.contains(c)).cloned().collect();
        for c in unchecked {
            if !s.sync_off.contains(&c) {
                s.sync_off.push(c);
            }
        }
        s.sync_off.retain(|c| !cats.contains(c));
        s.sync_cats = cats;
    }
    changed(&app, true);
}

#[tauri::command]
pub(crate) fn sync_set_lan(app: AppHandle, on: bool) {
    app.state::<Store>().settings.lock().unwrap().sync_lan = on;
    *app.state::<SyncRt>().lan_since.lock().unwrap() = on.then(Instant::now);
    changed(&app, false);
}

/// 신호가 닿지 않을 때 상대 주소를 직접 넣는다 (빈 값 = 자동으로 찾기만)
#[tauri::command]
pub(crate) fn sync_set_peer(app: AppHandle, peer: String) -> Result<(), String> {
    let peer = peer.trim().to_string();
    let n = peer.split([',', ' ', ';']).filter(|x| !x.trim().is_empty()).count();
    if manual_peers(&peer).len() != n {
        return Err("주소 형식이 맞지 않아요. 다른 컴퓨터의 설정에 표시된 주소(예: 192.168.0.12)를 넣어 주세요.".into());
    }
    app.state::<Store>().settings.lock().unwrap().sync_peer = peer;
    changed(&app, false);
    Ok(())
}

#[tauri::command]
pub(crate) async fn sync_pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = app.dialog().file()
        .set_title("동기화 폴더 고르기 (두 컴퓨터가 함께 보는 폴더)")
        .blocking_pick_folder() else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?.display().to_string();
    {
        let store = app.state::<Store>();
        let mut s = store.settings.lock().unwrap();
        s.sync_folder = path.clone();
        s.sync_folder_on = true;
    }
    changed(&app, true);
    Ok(Some(path))
}

#[tauri::command]
pub(crate) fn sync_set_folder_on(app: AppHandle, on: bool) {
    app.state::<Store>().settings.lock().unwrap().sync_folder_on = on;
    changed(&app, false);
}

/// "지금 맞추기": 상대·폴더 파일을 처음부터 다시 확인
#[tauri::command]
pub(crate) fn sync_now(app: AppHandle) {
    changed(&app, true);
}

// ──────── 테스트 ────────

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(uid: &str, text: &str, cat: &str, updated: i64) -> Memo {
        let mut m: Memo = serde_json::from_value(serde_json::json!({
            "text": text, "category": cat, "status": "todo", "done_at": null,
        })).unwrap();
        m.uid = uid.into();
        m.updated = updated;
        m.hash = memo_hash(&m);
        m
    }

    fn data(memos: Vec<Memo>) -> AppData {
        let mut d = AppData::default();
        for (i, mut m) in memos.into_iter().enumerate() {
            m.id = i as u64 + 1;
            d.memos.push(m);
        }
        d
    }

    fn doc(memos: Vec<Memo>) -> SyncDoc {
        SyncDoc { device: "other".into(), memos, ..Default::default() }
    }

    fn cats() -> Vec<String> {
        vec!["일반".into()]
    }

    #[test]
    fn newer_wins_and_local_id_kept() {
        let mut d = data(vec![memo("a", "old", "일반", 100)]);
        let (ch, _) = merge_into(&mut d, &mut cats(), &[], doc(vec![memo("a", "new", "일반", 200)]), &|_| false);
        assert!(ch);
        assert_eq!(d.memos[0].text, "new");
        assert_eq!(d.memos[0].id, 1);
        // 더 오래된 내용은 무시
        let (ch, _) = merge_into(&mut d, &mut cats(), &[], doc(vec![memo("a", "older", "일반", 150)]), &|_| false);
        assert!(!ch);
        assert_eq!(d.memos[0].text, "new");
    }

    #[test]
    fn receives_sent_categories_unless_turned_off() {
        // 받는 쪽은 아무 것도 고르지 않아도 상대가 보낸 카테고리를 받고, 그 카테고리를 함께 맞추게 된다
        let mut d = data(vec![]);
        let mut sc = vec![];
        let (ch, cc) = merge_into(&mut d, &mut sc, &[], doc(vec![memo("a", "x", "일반", 1), memo("b", "y", "회사", 1)]), &|_| false);
        assert!(ch && cc);
        assert_eq!(d.memos.len(), 2);
        assert!(sc.contains(&"회사".to_string()) && sc.contains(&"일반".to_string()));
        assert!(d.categories.contains(&"회사".to_string()));
        // 받는 쪽에서 체크를 끈 카테고리는 받지 않음
        let mut d = data(vec![]);
        let mut sc = vec![];
        merge_into(&mut d, &mut sc, &["회사".to_string()], doc(vec![memo("a", "x", "일반", 1), memo("b", "y", "회사", 1)]), &|_| false);
        assert_eq!(d.memos.len(), 1);
        assert!(!sc.contains(&"회사".to_string()));
        // 잠겨 있는 카테고리도 받지 않음
        let mut d = data(vec![]);
        let mut sc = vec![];
        merge_into(&mut d, &mut sc, &[], doc(vec![memo("b", "y", "회사", 1)]), &|c| c == "회사");
        assert!(d.memos.is_empty() && sc.is_empty());
        // 이미 같은 항목을 갖고 있어도 그 카테고리를 함께 맞추게 됨 (이쪽 수정도 보내지도록)
        let mut d = data(vec![memo("a", "x", "업무", 5)]);
        let mut sc = vec![];
        let (_, cc) = merge_into(&mut d, &mut sc, &[], doc(vec![memo("a", "x", "업무", 5)]), &|_| false);
        assert!(cc && sc == vec!["업무".to_string()]);
    }

    #[test]
    fn local_addresses_have_broadcast() {
        for (ip, b) in local_v4() {
            assert!(!ip.is_loopback());
            assert_ne!(b, ip);
        }
        assert_eq!(manual_peers("192.168.0.2, 10.0.0.5"), vec!["192.168.0.2".parse::<IpAddr>().unwrap(), "10.0.0.5".parse().unwrap()]);
    }

    #[test]
    fn same_doc_twice_is_no_change() {
        let mut d = data(vec![]);
        merge_into(&mut d, &mut cats(), &[], doc(vec![memo("a", "x", "일반", 1)]), &|_| false);
        let (ch, _) = merge_into(&mut d, &mut cats(), &[], doc(vec![memo("a", "x", "일반", 1)]), &|_| false);
        assert!(!ch);
        assert_eq!(d.memos.len(), 1);
    }

    #[test]
    fn tombstone_removes_older_but_not_newer_edit() {
        let mut d = data(vec![memo("a", "x", "일반", 100), memo("b", "y", "일반", 300)]);
        let mut del = doc(vec![]);
        del.deleted.insert("a".into(), 200);
        del.deleted.insert("b".into(), 200);
        merge_into(&mut d, &mut cats(), &[], del, &|_| false);
        assert_eq!(d.memos.len(), 1);
        assert_eq!(d.memos[0].uid, "b");
        // 지운 뒤에 예전 내용이 다시 와도 되살아나지 않음
        merge_into(&mut d, &mut cats(), &[], doc(vec![memo("a", "x", "일반", 100)]), &|_| false);
        assert!(!d.memos.iter().any(|m| m.uid == "a"));
    }

    #[test]
    fn moved_to_unsynced_category() {
        let mut d = data(vec![memo("a", "x", "일반", 100)]);
        let mut mv = doc(vec![]);
        mv.moved.push(Moved { uid: "a".into(), updated: 200, category: "회사".into() });
        merge_into(&mut d, &mut cats(), &[], mv, &|_| false);
        assert_eq!(d.memos[0].category, "회사");
        assert!(d.categories.contains(&"회사".to_string()));
    }

    #[test]
    fn rename_keeps_syncing() {
        let mut d = data(vec![memo("a", "x", "일반", 100)]);
        let mut sc = cats();
        let (_, cc) = merge_into(&mut d, &mut sc, &[], doc(vec![memo("a", "x", "업무", 200)]), &|_| false);
        assert!(cc);
        assert!(sc.contains(&"업무".to_string()));
    }

    #[test]
    fn code_and_crypto() {
        let c = new_code();
        assert_eq!(normalize_code(&c.to_lowercase().replace('-', " ")), Some(c.clone()));
        assert_eq!(normalize_code("ABCD-EFGH"), None);
        let k = key_from_code(&c);
        let sealed = seal(&k, b"hello");
        assert_eq!(open(&k, &sealed).as_deref(), Some(&b"hello"[..]));
        assert_eq!(open(&key_from_code(&new_code()), &sealed), None);
        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(open(&k, &bad), None);
    }
}

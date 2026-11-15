use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartManagerExt};
use tauri_plugin_updater::UpdaterExt;

mod crypto;
mod lock;
mod sync;

// ════════ 데이터 타입 ════════

/// 항목(메모)에 딸린 상세 메모 한 줄
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Note {
    id: u64,
    text: String,
    #[serde(default)]
    created: String,
    /// 상세 메모를 한 번 더 열었을 때(메모 창) 쓰는 본문
    #[serde(default)]
    body: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Memo {
    /// 메모장 탭이 순서 변경/삭제와 무관하게 항목을 가리키기 위한 고유 번호
    #[serde(default)]
    id: u64,
    text: String,
    #[serde(default)]
    description: String,
    category: String,
    status: String,
    #[serde(default)]
    done: bool,
    done_at: Option<String>,
    #[serde(default)]
    prev_status: Option<String>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    notes: Vec<Note>,
    /// 기기를 넘나들어도 같은 항목을 가리키는 고유 ID (동기화용)
    #[serde(default)]
    uid: String,
    /// 마지막으로 내용이 바뀐 시각 (밀리초). 동기화할 때 더 최근 것을 남긴다.
    #[serde(default)]
    updated: i64,
    /// 내용 지문 — 이게 달라지면 updated를 새로 찍는다 (stamp 참고)
    #[serde(default)]
    hash: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AppData {
    categories: Vec<String>,
    memos: Vec<Memo>,
    #[serde(default)]
    collapsed: HashMap<String, bool>,
    /// 완전히 지운 항목 uid → 지운 시각. 다른 기기에서도 지워지도록 남겨 둔다.
    #[serde(default)]
    deleted: HashMap<String, i64>,
    /// 잠근 카테고리의 암호화된 내용 (잠겨 있을 때만)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vault: Option<String>,
    /// 잠긴 항목들 중 가장 큰 id — 잠긴 동안 새 항목이 같은 id를 쓰지 않도록
    #[serde(default)]
    hidden_max_id: u64,
    /// 잠긴 항목의 id — 메모장 탭·고정 메모 창이 "지워진 것"으로 오해해 정리하지 않도록
    #[serde(default)]
    hidden_ids: Vec<u64>,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            categories: vec!["일반".into(), "개발".into(), "회의".into(), "기타".into()],
            memos: vec![],
            collapsed: HashMap::new(),
            deleted: HashMap::new(),
            vault: None,
            hidden_max_id: 0,
            hidden_ids: vec![],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct WinGeo {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// 고정된 상세 메모 창
#[derive(Serialize, Deserialize, Clone, Debug)]
struct NoteWin {
    memo_id: u64,
    note_id: u64,
    #[serde(default)]
    geo: WinGeo,
}

fn default_opacity() -> f64 { 1.0 }
fn default_notes_geo() -> WinGeo { WinGeo { x: 1248.0, y: 44.0, w: 420.0, h: 480.0 } }

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Settings {
    font_family: String,
    font_size: u32,
    panel: WinGeo,
    todo: WinGeo,
    doing: WinGeo,
    done: WinGeo,
    #[serde(default)]
    trash: WinGeo,
    #[serde(default)]
    calendar: WinGeo,
    #[serde(default)]
    autostart: bool,
    #[serde(default)]
    close_to_tray: bool,
    #[serde(default = "default_opacity")]
    opacity: f64,
    #[serde(default)]
    recent_fonts: Vec<String>,
    #[serde(default)]
    calendar_color: String,
    #[serde(default = "default_notes_geo")]
    notes: WinGeo,
    /// "다시 보지 않기"를 누른 마지막 버전 (빈 값 = 아직 안내를 닫은 적 없음)
    #[serde(default)]
    seen_version: String,
    #[serde(default)]
    pinned_notes: Vec<NoteWin>,
    // ── 자동 동기화 ──
    #[serde(default)]
    device_id: String,
    /// 같은 코드를 넣은 기기끼리 동기화된다. 빈 값 = 동기화 안 함
    #[serde(default)]
    sync_code: String,
    #[serde(default)]
    sync_cats: Vec<String>,
    /// 체크를 끈 카테고리 — 상대가 보내도 받지 않는다
    #[serde(default)]
    sync_off: Vec<String>,
    #[serde(default)]
    sync_lan: bool,
    /// 직접 넣은 상대 주소 (신호가 닿지 않는 네트워크용)
    #[serde(default)]
    sync_peer: String,
    #[serde(default)]
    sync_folder: String,
    #[serde(default)]
    sync_folder_on: bool,
    /// 잠금 (카테고리 숨기기) 설정. None = 사용 안 함
    #[serde(default)]
    lock: Option<lock::LockCfg>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: "맑은 고딕".into(),
            font_size: 14,
            panel: WinGeo { x: 420.0, y: 428.0, w: 573.0, h: 379.0 },
            todo: WinGeo { x: 420.0, y: 44.0, w: 403.0, h: 374.0 },
            doing: WinGeo { x: 833.0, y: 44.0, w: 405.0, h: 372.0 },
            done: WinGeo { x: 420.0, y: 486.0, w: 404.0, h: 371.0 },
            trash: WinGeo { x: 833.0, y: 486.0, w: 404.0, h: 371.0 },
            // 세로는 첫 렌더링 때 "그 달의 날짜가 다 보이는 최소 높이"로 자동 조정됨
            calendar: WinGeo { x: 2.0, y: 44.0, w: 408.0, h: 300.0 },
            autostart: true,
            close_to_tray: true,
            opacity: 1.0,
            recent_fonts: vec![],
            calendar_color: String::new(),
            notes: default_notes_geo(),
            seen_version: String::new(),
            pinned_notes: vec![],
            device_id: String::new(),
            sync_code: String::new(),
            sync_cats: vec![],
            sync_off: vec![],
            sync_lan: false,
            sync_peer: String::new(),
            sync_folder: String::new(),
            sync_folder_on: false,
            lock: None,
        }
    }
}

// ════════ 상태 저장소 ════════

struct Store {
    data: Mutex<AppData>,
    settings: Mutex<Settings>,
    data_path: PathBuf,
    settings_path: PathBuf,
    /// 처음 설치해서 저장된 파일이 하나도 없던 경우
    fresh_install: bool,
    /// 메모장 창이 아직 로드되지 않았을 때 열어야 할 항목 id
    pending_note: Mutex<Option<u64>>,
    /// 이 기기의 고유 ID (uid 만들 때 사용)
    device: String,
    /// 잠금이 풀려 있는 동안의 데이터 키 (잠겨 있으면 None)
    lock_key: Mutex<Option<[u8; 32]>>,
    /// 마지막으로 창을 만진 시각 (자동 다시 잠금용)
    last_active: Mutex<Instant>,
}

impl Store {
    fn new(dir: PathBuf) -> Self {
        let dp = dir.join("memos.json");
        let sp = dir.join("settings.json");
        let fresh_install = !dp.exists() && !sp.exists();
        let mut data: AppData = fs::read_to_string(&dp)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // 예전 버전 데이터에는 id가 없으므로 여기서 부여
        let mut next = max_id(&data);
        for m in data.memos.iter_mut().filter(|m| m.id == 0) {
            next += 1;
            m.id = next;
        }
        let mut settings: Settings = fs::read_to_string(&sp)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let new_device = settings.device_id.is_empty();
        if new_device {
            settings.device_id = random_hex(8);
        }
        let device = settings.device_id.clone();
        // 예전 버전 데이터에는 uid/수정 시각이 없으므로 여기서 부여
        stamp(&mut data, &device);
        let lock_key = lock::on_load(&mut data, &mut settings);
        let store = Self {
            data: Mutex::new(data),
            settings: Mutex::new(settings),
            data_path: dp,
            settings_path: sp,
            fresh_install,
            pending_note: Mutex::new(None),
            device,
            lock_key: Mutex::new(lock_key),
            last_active: Mutex::new(Instant::now()),
        };
        if new_device {
            store.save_settings();
        }
        if !fresh_install {
            store.save_data();
        }
        store
    }

    fn save_data(&self) {
        if let Ok(d) = self.data.lock() {
            // 잠금을 쓰면 잠근 카테고리는 암호화해서 쓴다
            let out = lock::for_disk(&d, &self.settings.lock().unwrap(), *self.lock_key.lock().unwrap());
            if let Ok(j) = serde_json::to_string_pretty(&out) {
                let _ = fs::write(&self.data_path, j);
            }
        }
    }

    fn save_settings(&self) {
        if let Ok(s) = self.settings.lock() {
            if let Ok(j) = serde_json::to_string_pretty(&lock::settings_for_disk(&s)) {
                let _ = fs::write(&self.settings_path, j);
            }
        }
    }
}

// ════════ 동기화용 기록 (uid · 수정 시각 · 삭제 기록) ════════

/// 새 항목에 쓸 id의 기준 (잠긴 항목의 id까지 고려)
fn max_id(d: &AppData) -> u64 {
    d.memos.iter().map(|m| m.id).max().unwrap_or(0).max(d.hidden_max_id)
}

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 항목 내용의 지문 (FNV-1a). 앱 버전이 바뀌어도 값이 같아야 하므로 직접 구현.
fn memo_hash(m: &Memo) -> u64 {
    let mut c = m.clone();
    c.id = 0;
    c.updated = 0;
    c.hash = 0;
    let json = serde_json::to_vec(&c).unwrap_or_default();
    json.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// 삭제 기록은 이 기간이 지나면 정리한다
const TOMBSTONE_KEEP_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// 내용이 바뀐 항목에 수정 시각을 찍고, uid가 없는 항목에는 uid를 붙인다.
/// 모든 변경은 emit_refresh를 거치므로 명령마다 따로 챙길 필요가 없다.
fn stamp(d: &mut AppData, device: &str) {
    let now = now_ms();
    for m in d.memos.iter_mut() {
        if m.uid.is_empty() {
            m.uid = format!("{device}-{}-{}", random_hex(4), m.id);
        }
        let h = memo_hash(m);
        if h != m.hash {
            m.hash = h;
            m.updated = now;
        }
    }
    d.deleted.retain(|_, t| now - *t < TOMBSTONE_KEEP_MS);
}

fn tombstone(d: &mut AppData, uid: &str) {
    if !uid.is_empty() {
        d.deleted.insert(uid.to_string(), now_ms());
    }
}

/// 데이터 변경 후 모든 창에 이벤트 전송 + 파일 저장
fn emit_refresh(app: &AppHandle, store: &State<Store>) {
    if let Ok(mut d) = store.data.lock() {
        stamp(&mut d, &store.device);
    }
    store.save_data();
    if let Some(rt) = app.try_state::<sync::SyncRt>() {
        rt.bump();
    }
    if let Ok(d) = store.data.lock() {
        let _ = app.emit("data-changed", &*d);
    }
}

// ════════ 데이터 명령 ════════

#[tauri::command]
fn get_all_data(store: State<Store>) -> AppData {
    store.data.lock().unwrap().clone()
}

#[tauri::command]
fn get_categories(store: State<Store>) -> Vec<String> {
    store.data.lock().unwrap().categories.clone()
}

#[tauri::command]
fn get_settings(store: State<Store>) -> Settings {
    store.settings.lock().unwrap().clone()
}

#[tauri::command]
fn add_memo(app: AppHandle, store: State<Store>, text: String, desc: String, category: String, status: String, due_date: Option<String>, end_date: Option<String>) {
    let end_date = end_date.filter(|s| !s.is_empty());
    let mut d = store.data.lock().unwrap();
    let id = max_id(&d) + 1;
    d.memos.push(Memo {
        id, text, description: desc, category, status,
        done: false, done_at: None, prev_status: None, due_date, end_date,
        notes: vec![], uid: String::new(), updated: 0, hash: 0,
    });
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn edit_memo(app: AppHandle, store: State<Store>, idx: usize, text: String, desc: String, category: Option<String>, due_date: Option<String>, end_date: Option<String>) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.text = text;
        m.description = desc;
        if let Some(cat) = category {
            m.category = cat;
        }
        m.due_date = due_date;
        // end_date는 인자가 전달된 경우에만 갱신 (todo/doing 창의 수정으로 기간이 지워지지 않도록)
        if let Some(ed) = end_date {
            m.end_date = if ed.is_empty() { None } else { Some(ed) };
        }
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn move_memo(app: AppHandle, store: State<Store>, idx: usize, target: String) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.status = target;
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn complete_memo(app: AppHandle, store: State<Store>, idx: usize) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.done = true;
        m.status = "done".into();
        m.done_at = Some(Local::now().format("%Y-%m-%d %H:%M").to_string());
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn restore_memo(app: AppHandle, store: State<Store>, idx: usize) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.status = m.prev_status.take().unwrap_or_else(|| "todo".into());
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn uncomplete_memo(app: AppHandle, store: State<Store>, idx: usize, target: String) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.done = false;
        m.status = target;
        m.done_at = None;
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn delete_memo(app: AppHandle, store: State<Store>, idx: usize) {
    // 휴지통으로 이동 (소프트 삭제)
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.prev_status = Some(m.status.clone());
        m.status = "trash".into();
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn permanent_delete(app: AppHandle, store: State<Store>, idx: usize) {
    let mut d = store.data.lock().unwrap();
    if idx < d.memos.len() {
        let m = d.memos.remove(idx);
        tombstone(&mut d, &m.uid);
    }
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn empty_trash(app: AppHandle, store: State<Store>) {
    let mut d = store.data.lock().unwrap();
    let gone: Vec<String> = d.memos.iter().filter(|m| m.status == "trash").map(|m| m.uid.clone()).collect();
    d.memos.retain(|m| m.status != "trash");
    for u in gone { tombstone(&mut d, &u); }
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn toggle_collapse(app: AppHandle, store: State<Store>, key: String) {
    let mut d = store.data.lock().unwrap();
    let v = d.collapsed.entry(key).or_insert(false);
    *v = !*v;
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn move_memo_position(app: AppHandle, store: State<Store>, from: usize, to: usize) {
    let mut d = store.data.lock().unwrap();
    if from < d.memos.len() && to < d.memos.len() && from != to {
        let memo = d.memos.remove(from);
        d.memos.insert(to, memo);
    }
    drop(d);
    emit_refresh(&app, &store);
}

// ════════ 상세 메모(메모장) 명령 ════════

#[tauri::command]
fn add_note(app: AppHandle, store: State<Store>, memo_id: u64, text: String) -> u64 {
    let mut d = store.data.lock().unwrap();
    let Some(m) = d.memos.iter_mut().find(|m| m.id == memo_id) else { return 0 };
    let id = m.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    m.notes.push(Note { id, text, created: Local::now().format("%Y-%m-%d %H:%M").to_string(), body: String::new() });
    drop(d);
    emit_refresh(&app, &store);
    id
}

#[tauri::command]
fn edit_note(app: AppHandle, store: State<Store>, memo_id: u64, note_id: u64, text: String) {
    let mut d = store.data.lock().unwrap();
    if let Some(n) = d.memos.iter_mut().find(|m| m.id == memo_id)
        .and_then(|m| m.notes.iter_mut().find(|n| n.id == note_id)) {
        if n.text == text { return; }
        n.text = text;
    }
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn delete_note(app: AppHandle, store: State<Store>, memo_id: u64, note_id: u64) {
    let mut d = store.data.lock().unwrap();
    if let Some(m) = d.memos.iter_mut().find(|m| m.id == memo_id) {
        m.notes.retain(|n| n.id != note_id);
    }
    drop(d);
    emit_refresh(&app, &store);
}

/// 메모장에서 상세 메모 줄을 끌어서 순서를 바꿨을 때: order는 메모 id를 새 순서대로 나열한 것
#[tauri::command]
fn reorder_notes(app: AppHandle, store: State<Store>, memo_id: u64, order: Vec<u64>) {
    let mut d = store.data.lock().unwrap();
    let Some(m) = d.memos.iter_mut().find(|m| m.id == memo_id) else { return };
    let before: Vec<u64> = m.notes.iter().map(|n| n.id).collect();
    // order에 없는 메모(그 사이 추가된 것 등)는 원래 순서대로 맨 뒤에 남긴다
    let pos = |id: u64| order.iter().position(|&o| o == id).unwrap_or(usize::MAX);
    m.notes.sort_by_key(|n| pos(n.id));
    if m.notes.iter().map(|n| n.id).eq(before.into_iter()) { return; }
    drop(d);
    emit_refresh(&app, &store);
}

/// 목록에서 항목을 눌렀을 때: 메모장 창을 띄우고 해당 항목 탭을 연다
#[tauri::command]
fn open_note(app: AppHandle, store: State<Store>, id: u64) {
    *store.pending_note.lock().unwrap() = Some(id);
    if let Some(w) = app.get_webview_window("notes") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    let _ = app.emit_to("notes", "open-note", id);
}

/// 메모장 창이 처음 로드될 때, 그 전에 요청된 항목이 있으면 가져간다
#[tauri::command]
fn take_pending_note(store: State<Store>) -> Option<u64> {
    store.pending_note.lock().unwrap().take()
}

#[tauri::command]
fn edit_note_body(app: AppHandle, store: State<Store>, memo_id: u64, note_id: u64, body: String) {
    let mut d = store.data.lock().unwrap();
    if let Some(n) = d.memos.iter_mut().find(|m| m.id == memo_id)
        .and_then(|m| m.notes.iter_mut().find(|n| n.id == note_id)) {
        if n.body == body { return; }
        n.body = body;
    }
    drop(d);
    emit_refresh(&app, &store);
}

// ════════ 상세 메모 창 (메모장에서 한 번 더 열기) ════════
// 창 라벨: note-{항목 id}-{메모 id}
// 고정하지 않은 창은 다른 메모를 열거나 메모장에서 다른 탭을 누르면 닫힌다.

fn note_label(memo_id: u64, note_id: u64) -> String {
    format!("note-{memo_id}-{note_id}")
}

fn is_pinned_label(store: &Store, label: &str) -> bool {
    store.settings.lock().unwrap().pinned_notes.iter()
        .any(|p| note_label(p.memo_id, p.note_id) == label)
}

fn win_geo(w: &tauri::WebviewWindow) -> Option<WinGeo> {
    let sf = w.scale_factor().unwrap_or(1.0);
    let (pos, size) = (w.outer_position().ok()?, w.inner_size().ok()?);
    Some(WinGeo {
        x: pos.x as f64 / sf,
        y: pos.y as f64 / sf,
        w: size.width as f64 / sf,
        h: size.height as f64 / sf,
    })
}

fn close_unpinned_notes(app: &AppHandle, except: Option<&str>) {
    let store = app.state::<Store>();
    for (label, w) in app.webview_windows() {
        if label.starts_with("note-") && Some(label.as_str()) != except && !is_pinned_label(&store, &label) {
            let _ = w.close();
        }
    }
}

fn build_note_window(app: &AppHandle, memo_id: u64, note_id: u64, geo: &WinGeo) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, note_label(memo_id, note_id), WebviewUrl::App("note.html".into()))
        .title("상세 메모")
        .decorations(false)
        .inner_size(if geo.w > 0.0 { geo.w } else { 320.0 }, if geo.h > 0.0 { geo.h } else { 340.0 })
        .position(geo.x, geo.y)
        .min_inner_size(220.0, 160.0)
        .build()?;
    let opacity = app.state::<Store>().settings.lock().unwrap().opacity;
    if opacity < 1.0 {
        apply_window_opacity(app, opacity);
    }
    Ok(())
}

fn emit_note_pins(app: &AppHandle) {
    let pins = app.state::<Store>().settings.lock().unwrap().pinned_notes.clone();
    let _ = app.emit("note-pins-changed", &pins);
}

/// 메모장의 상세 메모를 눌렀을 때: 그 메모만 따로 창으로 연다 (창 생성은 async 명령에서)
#[tauri::command]
async fn open_note_window(app: AppHandle, memo_id: u64, note_id: u64) -> Result<(), String> {
    let label = note_label(memo_id, note_id);
    close_unpinned_notes(&app, Some(&label));
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    // 메모장 창 오른쪽에 붙여서 띄우고, 여러 개면 조금씩 어긋나게
    let n = app.webview_windows().keys().filter(|l| l.starts_with("note-")).count() as f64;
    let geo = app.get_webview_window("notes").and_then(|w| win_geo(&w))
        .map(|g| WinGeo { x: g.x + g.w + 8.0 + n * 24.0, y: g.y + n * 24.0, w: 320.0, h: 340.0 })
        .unwrap_or(WinGeo { x: 300.0 + n * 24.0, y: 120.0 + n * 24.0, w: 320.0, h: 340.0 });
    build_note_window(&app, memo_id, note_id, &geo).map_err(|e| e.to_string())
}

/// 메모장에서 다른 탭(항목)으로 넘어갈 때 고정 안 된 메모 창 닫기
#[tauri::command]
fn close_unpinned_note_windows(app: AppHandle) {
    close_unpinned_notes(&app, None);
}

#[tauri::command]
fn get_note_pins(store: State<Store>) -> Vec<NoteWin> {
    store.settings.lock().unwrap().pinned_notes.clone()
}

/// 메모 창의 고정 버튼. 고정된 창은 다음 실행 때도 그 자리에 다시 뜬다.
#[tauri::command]
fn toggle_note_pin(app: AppHandle, window: tauri::WebviewWindow, store: State<Store>, memo_id: u64, note_id: u64) -> bool {
    let mut s = store.settings.lock().unwrap();
    let pinned = if let Some(i) = s.pinned_notes.iter().position(|p| p.memo_id == memo_id && p.note_id == note_id) {
        s.pinned_notes.remove(i);
        false
    } else {
        let geo = win_geo(&window).unwrap_or_default();
        s.pinned_notes.push(NoteWin { memo_id, note_id, geo });
        true
    };
    drop(s);
    store.save_settings();
    emit_note_pins(&app);
    pinned
}

/// 메모 창 닫기 (X 버튼, 또는 메모가 삭제된 경우). 고정되어 있었다면 고정도 해제.
#[tauri::command]
fn close_note_window(app: AppHandle, window: tauri::WebviewWindow, store: State<Store>) {
    let label = window.label().to_string();
    let mut s = store.settings.lock().unwrap();
    let before = s.pinned_notes.len();
    s.pinned_notes.retain(|p| note_label(p.memo_id, p.note_id) != label);
    let changed = s.pinned_notes.len() != before;
    drop(s);
    if changed {
        store.save_settings();
        emit_note_pins(&app);
    }
    let _ = window.close();
}

// ════════ 사용법 / 새 기능 안내 ════════

/// 안내 기능이 처음 들어간 버전 바로 전 버전.
/// 이 버전에서 업데이트한 사용자는 "새로 추가된 내용"만 보게 된다.
const LAST_VERSION_WITHOUT_GUIDE: &str = "1.1.3";

#[derive(Serialize)]
struct GuideState {
    version: String,
    /// None이면 처음 설치 → 사용법 전체, Some(v)면 v보다 새 버전 내용만
    since: Option<String>,
}

#[tauri::command]
fn get_guide_state(app: AppHandle, store: State<Store>) -> GuideState {
    let seen = store.settings.lock().unwrap().seen_version.clone();
    let since = if !seen.is_empty() {
        Some(seen)
    } else if store.fresh_install {
        None
    } else {
        Some(LAST_VERSION_WITHOUT_GUIDE.to_string())
    };
    GuideState { version: app.package_info().version.to_string(), since }
}

/// "다시 보지 않기": 현재 버전까지 본 것으로 기록
#[tauri::command]
fn dismiss_guide(app: AppHandle, store: State<Store>) {
    store.settings.lock().unwrap().seen_version = app.package_info().version.to_string();
    store.save_settings();
    if let Some(w) = app.get_webview_window("guide") {
        let _ = w.hide();
    }
}

/// 패널/트레이에서 "사용법" 버튼: 언제든 전체 사용법 보기
#[tauri::command]
fn open_guide(app: AppHandle) {
    show_full_guide(&app);
}

fn show_full_guide(app: &AppHandle) {
    let _ = app.emit_to("guide", "guide-show-all", ());
    if let Some(w) = app.get_webview_window("guide") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ════════ 카테고리 명령 ════════

#[tauri::command]
fn add_category(app: AppHandle, store: State<Store>, name: String) -> bool {
    let name = name.trim().to_string();
    if name.is_empty() || lock::is_hidden(&store, &name) { return false; }
    let mut d = store.data.lock().unwrap();
    if d.categories.contains(&name) { return false; }
    d.categories.push(name);
    drop(d);
    emit_refresh(&app, &store);
    true
}

#[tauri::command]
fn delete_category(app: AppHandle, store: State<Store>, name: String) {
    let mut d = store.data.lock().unwrap();
    d.categories.retain(|c| c != &name);
    let fb = d.categories.first().cloned().unwrap_or_else(|| "일반".into());
    if d.categories.is_empty() { d.categories.push("일반".into()); }
    for m in &mut d.memos {
        if m.category == name { m.category = fb.clone(); }
    }
    drop(d);
    let mut s = store.settings.lock().unwrap();
    if s.sync_cats.contains(&name) || s.sync_off.contains(&name) {
        s.sync_cats.retain(|c| c != &name);
        s.sync_off.retain(|c| c != &name);
        drop(s);
        store.save_settings();
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn rename_category(app: AppHandle, store: State<Store>, old: String, new_name: String) {
    let nn = new_name.trim().to_string();
    if lock::is_hidden(&store, &nn) { return; }
    let mut d = store.data.lock().unwrap();
    if nn.is_empty() || !d.categories.contains(&old) || d.categories.contains(&nn) { return; }
    if let Some(pos) = d.categories.iter().position(|c| c == &old) {
        d.categories[pos] = nn.clone();
    }
    for m in &mut d.memos {
        if m.category == old { m.category = nn.clone(); }
    }
    let keys: Vec<_> = d.collapsed.keys()
        .filter(|k| k.ends_with(&format!(":{old}")))
        .cloned().collect();
    for k in keys {
        let v = d.collapsed.remove(&k).unwrap_or(false);
        d.collapsed.insert(k.replace(&format!(":{old}"), &format!(":{nn}")), v);
    }
    drop(d);
    // 동기화 중인 카테고리면 새 이름으로 계속 동기화
    let mut guard = store.settings.lock().unwrap();
    let s = &mut *guard;
    let mut renamed = false;
    for c in s.sync_cats.iter_mut().chain(s.sync_off.iter_mut()) {
        if *c == old {
            *c = nn.clone();
            renamed = true;
        }
    }
    if renamed {
        drop(guard);
        store.save_settings();
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn move_category(app: AppHandle, store: State<Store>, name: String, direction: String) {
    let mut d = store.data.lock().unwrap();
    if let Some(pos) = d.categories.iter().position(|c| c == &name) {
        let new_pos = if direction == "up" {
            if pos == 0 { return; }
            pos - 1
        } else {
            if pos >= d.categories.len() - 1 { return; }
            pos + 1
        };
        d.categories.swap(pos, new_pos);
    }
    drop(d);
    emit_refresh(&app, &store);
}

// ════════ 데이터 옮기기 (다른 컴퓨터로 내보내기 / 가져오기) ════════

/// 내보낸 파일 형식
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExportFile {
    #[serde(default)]
    app: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    exported: String,
    categories: Vec<String>,
    memos: Vec<Memo>,
}

#[derive(Serialize)]
struct ImportResult {
    added: usize,
    skipped: usize,
    removed: usize,
}

/// 고른 카테고리의 항목(휴지통 제외)을 파일로 저장. 취소하면 None.
#[tauri::command]
async fn export_data(app: AppHandle, store: State<'_, Store>, categories: Vec<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file = {
        let d = store.data.lock().unwrap();
        ExportFile {
            app: "Todi".into(),
            version: app.package_info().version.to_string(),
            exported: Local::now().format("%Y-%m-%d %H:%M").to_string(),
            // 순서는 이 컴퓨터의 카테고리 순서대로
            categories: d.categories.iter().filter(|c| categories.contains(c)).cloned().collect(),
            memos: d.memos.iter()
                .filter(|m| m.status != "trash" && categories.contains(&m.category))
                .cloned().collect(),
        }
    };
    let name = format!("Todi-{}.json", Local::now().format("%Y%m%d"));
    let Some(path) = app.dialog().file()
        .set_title("Todi 데이터 내보내기")
        .add_filter("Todi 데이터", &["json"])
        .set_file_name(&name)
        .blocking_save_file() else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("파일을 저장하지 못했어요: {e}"))?;
    Ok(Some(path.display().to_string()))
}

/// 가져올 파일을 골라서 내용을 돌려준다 (실제 반영은 import_data). 취소하면 None.
#[tauri::command]
async fn open_import_file(app: AppHandle) -> Result<Option<ExportFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = app.dialog().file()
        .set_title("Todi 데이터 불러오기")
        .add_filter("Todi 데이터", &["json"])
        .blocking_pick_file() else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let text = fs::read_to_string(&path).map_err(|e| format!("파일을 열지 못했어요: {e}"))?;
    // 내보낸 파일뿐 아니라 memos.json(전체 데이터 파일)을 직접 골라도 읽을 수 있다
    serde_json::from_str::<ExportFile>(&text)
        .map(Some)
        .map_err(|_| "Todi 데이터 파일이 아니에요.".to_string())
}

/// mode: "merge" = 기존 항목은 두고 없는 항목만 추가, "replace" = 고른 카테고리의 기존 항목을 파일 내용으로 교체
#[tauri::command]
fn import_data(app: AppHandle, store: State<Store>, file: ExportFile, categories: Vec<String>, mode: String) -> ImportResult {
    let replace = mode == "replace";
    // 잠겨 있는 카테고리에는 지금 넣을 수 없음 (잠긴 내용과 섞이지 않도록)
    let categories: Vec<String> = categories.into_iter().filter(|c| !lock::is_hidden(&store, c)).collect();
    let res = import_into(&mut store.data.lock().unwrap(), file, &categories, replace);
    emit_refresh(&app, &store);
    res
}

/// 불러오기의 실제 반영 (화면·저장과 떼어 놓아 테스트할 수 있게)
fn import_into(d: &mut AppData, file: ExportFile, categories: &[String], replace: bool) -> ImportResult {
    let mut res = ImportResult { added: 0, skipped: 0, removed: 0 };
    let mut reuse: HashMap<String, u64> = HashMap::new();
    for c in file.categories.iter().filter(|c| categories.contains(c)) {
        if !d.categories.contains(c) {
            d.categories.push(c.clone());
        }
    }
    let incoming: Vec<Memo> = file.memos.into_iter()
        .filter(|m| m.status != "trash" && categories.contains(&m.category)).collect();
    if replace {
        let keep: std::collections::HashSet<String> = incoming.iter()
            .map(|m| m.uid.clone()).filter(|u| !u.is_empty()).collect();
        let (gone, kept): (Vec<Memo>, Vec<Memo>) = std::mem::take(&mut d.memos).into_iter()
            .partition(|m| m.status != "trash" && categories.contains(&m.category));
        d.memos = kept;
        // 같은 항목을 다시 넣을 때는 이 컴퓨터의 id를 그대로 써서, 메모장 탭·고정 메모 창이 유지되게
        for m in &gone {
            if keep.contains(&m.uid) { reuse.insert(m.uid.clone(), m.id); }
        }
        res.removed = gone.len();
        // 지운 항목은 다른 기기에서도 지워지도록 삭제 기록 (같은 항목을 다시 넣는 경우는 제외)
        for m in &gone {
            if !keep.contains(&m.uid) { tombstone(d, &m.uid); }
        }
        // 휴지통에 같은 항목이 있으면 겹치지 않게 정리
        d.memos.retain(|m| !keep.contains(&m.uid));
    }
    let same = |a: &Memo, b: &Memo| a.category == b.category && a.text == b.text
        && a.description == b.description && a.due_date == b.due_date && a.end_date == b.end_date;
    let mut next = max_id(d);
    for m in incoming {
        if !replace {
            // 같은 항목(uid)이 이미 있으면 더 최근 것으로 맞추고, 내용이 같은 항목은 건너뜀
            if let Some(x) = d.memos.iter_mut().find(|x| !m.uid.is_empty() && x.uid == m.uid) {
                if m.updated > x.updated {
                    let id = x.id;
                    *x = Memo { id, ..m };
                    x.hash = memo_hash(x);
                    res.added += 1;
                } else {
                    res.skipped += 1;
                }
                continue;
            }
            if d.memos.iter().any(|x| x.status != "trash" && same(x, &m)) {
                res.skipped += 1;
                continue;
            }
        }
        let id = match reuse.get(&m.uid) {
            Some(&id) => id,
            None => { next += 1; next }
        };
        let mut m = Memo { id, ..m };
        m.hash = memo_hash(&m);
        d.memos.push(m);
        res.added += 1;
    }
    let live: std::collections::HashSet<String> = d.memos.iter().map(|m| m.uid.clone()).collect();
    d.deleted.retain(|u, _| !live.contains(u));
    res
}

// ════════ 설정 명령 ════════

#[tauri::command]
fn save_user_settings(app: AppHandle, store: State<Store>, font_family: String, font_size: u32) {
    let mut s = store.settings.lock().unwrap();
    s.recent_fonts.retain(|f| f != &font_family);
    s.recent_fonts.insert(0, font_family.clone());
    s.recent_fonts.truncate(8);
    s.font_family = font_family;
    s.font_size = font_size.clamp(10, 24);
    let settings_clone = s.clone();
    drop(s);
    store.save_settings();
    // 모든 창에 즉시 폰트 적용 이벤트
    let _ = app.emit("settings-changed", &settings_clone);
}

#[tauri::command]
fn reorder_categories(app: AppHandle, store: State<Store>, categories: Vec<String>) {
    let mut d = store.data.lock().unwrap();
    d.categories = categories;
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn get_system_fonts() -> Vec<String> {
    use font_kit::source::SystemSource;
    let source = SystemSource::new();
    let mut fonts = source.all_families().unwrap_or_default();
    fonts.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    fonts
}

#[tauri::command]
fn set_autostart(app: AppHandle, store: State<Store>, enabled: bool) {
    let autostart_manager = app.autolaunch();
    if enabled {
        let _ = autostart_manager.enable();
    } else {
        let _ = autostart_manager.disable();
    }
    let mut s = store.settings.lock().unwrap();
    s.autostart = enabled;
    drop(s);
    store.save_settings();
}

#[tauri::command]
fn set_opacity(app: AppHandle, store: State<Store>, value: f64) {
    let opacity = value.clamp(0.3, 1.0);
    apply_window_opacity(&app, opacity);
    let mut s = store.settings.lock().unwrap();
    s.opacity = opacity;
    drop(s);
    store.save_settings();
}

fn apply_window_opacity(app: &AppHandle, opacity: f64) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let mut labels: Vec<String> = ["panel", "todo", "doing", "done", "trash", "calendar", "notes"]
            .iter().map(|s| s.to_string()).collect();
        labels.extend(app.webview_windows().into_keys().filter(|l| l.starts_with("note-")));
        for label in labels {
            if let Some(w) = app.get_webview_window(&label) {
                if let Ok(hwnd) = w.hwnd() {
                    let h = hwnd.0 as *mut std::ffi::c_void;
                    unsafe {
                        let style = GetWindowLongW(h, GWL_EXSTYLE);
                        SetWindowLongW(h, GWL_EXSTYLE, style | WS_EX_LAYERED as i32);
                        SetLayeredWindowAttributes(h, 0, (opacity * 255.0) as u8, LWA_ALPHA);
                    }
                }
            }
        }
    }
}

#[tauri::command]
fn save_calendar_color(app: AppHandle, store: State<Store>, color: String) {
    let mut s = store.settings.lock().unwrap();
    s.calendar_color = color;
    let settings_clone = s.clone();
    drop(s);
    store.save_settings();
    let _ = app.emit("settings-changed", &settings_clone);
}

#[tauri::command]
fn set_close_to_tray(store: State<Store>, enabled: bool) {
    let mut s = store.settings.lock().unwrap();
    s.close_to_tray = enabled;
    drop(s);
    store.save_settings();
}

#[tauri::command]
fn close_window(app: AppHandle, store: State<Store>, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        // 숨기기 전에 위치/크기 저장
        let sf = w.scale_factor().unwrap_or(1.0);
        if let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) {
            let geo = WinGeo {
                x: pos.x as f64 / sf,
                y: pos.y as f64 / sf,
                w: size.width as f64 / sf,
                h: size.height as f64 / sf,
            };
            let mut s = store.settings.lock().unwrap();
            match label.as_str() {
                "panel" => s.panel = geo,
                "todo"  => s.todo = geo,
                "doing" => s.doing = geo,
                "done"  => s.done = geo,
                "trash" => s.trash = geo,
                "calendar" => s.calendar = geo,
                "notes" => s.notes = geo,
                _ => {}
            }
            drop(s);
            store.save_settings();
        }
        let _ = w.hide();
    }
}

#[tauri::command]
fn start_drag(window: tauri::WebviewWindow) {
    let _ = window.start_dragging();
}

// ════════ 창 관리 명령 ════════

#[tauri::command]
fn show_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn hide_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.hide();
    }
}

#[tauri::command]
fn minimize_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.minimize();
    }
}

/// 내용이 다 보이는 최소 높이를 창의 최소 크기로 지정하고,
/// 현재 높이가 그보다 짧으면 늘린다. (가로 크기는 그대로 유지)
#[tauri::command]
fn fit_window_height(app: AppHandle, label: String, height: f64) {
    if let Some(w) = app.get_webview_window(&label) {
        let need = height.clamp(180.0, 2000.0);
        let sf = w.scale_factor().unwrap_or(1.0);
        let _ = w.set_min_size(Some(tauri::LogicalSize::new(260.0, need)));
        if let Ok(size) = w.inner_size() {
            let cur_w = size.width as f64 / sf;
            let cur_h = size.height as f64 / sf;
            if cur_h + 0.5 < need {
                let _ = w.set_size(tauri::LogicalSize::new(cur_w, need));
            }
        }
    }
}

#[tauri::command]
fn toggle_on_top(app: AppHandle, label: String) -> bool {
    if let Some(w) = app.get_webview_window(&label) {
        let cur = w.is_always_on_top().unwrap_or(false);
        let _ = w.set_always_on_top(!cur);
        !cur
    } else {
        false
    }
}

/// 창 위치 저장 후 종료
fn save_and_exit(app: &AppHandle) {
    let store = app.state::<Store>();
    let mut s = store.settings.lock().unwrap();
    for p in s.pinned_notes.iter_mut() {
        if let Some(g) = app.get_webview_window(&note_label(p.memo_id, p.note_id)).and_then(|w| win_geo(&w)) {
            p.geo = g;
        }
    }
    for label in ["panel", "todo", "doing", "done", "trash", "calendar", "notes"] {
        if let Some(w) = app.get_webview_window(label) {
            // 숨겨진 창은 크기가 정확하지 않으므로 건너뛰기
            if !w.is_visible().unwrap_or(false) { continue; }
            // scale_factor로 나눠서 logical 좌표로 저장
            let sf = w.scale_factor().unwrap_or(1.0);
            if let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) {
                let geo = WinGeo {
                    x: pos.x as f64 / sf,
                    y: pos.y as f64 / sf,
                    w: size.width as f64 / sf,
                    h: size.height as f64 / sf,
                };
                match label {
                    "panel" => s.panel = geo,
                    "todo"  => s.todo = geo,
                    "doing" => s.doing = geo,
                    "done"  => s.done = geo,
                    "trash" => s.trash = geo,
                    "calendar" => s.calendar = geo,
                    "notes" => s.notes = geo,
                    _ => {}
                }
            }
        }
    }
    drop(s);
    store.save_data();
    store.save_settings();
    app.exit(0);
}

#[tauri::command]
fn close_app(app: AppHandle) {
    save_and_exit(&app);
}

// ════════ 앱 실행 ════════

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            // 데이터 디렉토리
            let data_dir = app.path().app_data_dir()?;
            // 개발용 빌드에서만: 실제 데이터를 건드리지 않고 시험할 수 있게 다른 폴더를 쓸 수 있다
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("TODI_DATA_DIR").map(std::path::PathBuf::from).unwrap_or(data_dir);
            fs::create_dir_all(&data_dir)?;

            let store = Store::new(data_dir);
            let settings = store.settings.lock().unwrap().clone();
            app.manage(store);
            app.manage(sync::SyncRt::new());
            sync::start(app.handle().clone());
            lock::start_idle_watch(app.handle().clone());

            // 창 생성: (label, html, geo, visible)
            let wins: Vec<(&str, &str, &WinGeo, bool)> = vec![
                ("panel", "index.html",  &settings.panel, true),
                ("todo",  "todo.html",   &settings.todo,  true),
                ("doing", "doing.html",  &settings.doing, true),
                ("done",  "done.html",   &settings.done,  false),
                ("trash", "trash.html",  &settings.trash,  false),
                ("calendar", "calendar.html", &settings.calendar, true),
                ("notes", "notes.html", &settings.notes, false),
            ];

            for (label, url, geo, visible) in wins {
                WebviewWindowBuilder::new(
                    app, label, WebviewUrl::App(url.into()),
                )
                .title(match label {
                    "panel" => "Todi",
                    "todo"  => "할 일",
                    "doing" => "하고 있는 일",
                    "done"  => "완료 목록",
                    "trash" => "휴지통",
                    "calendar" => "달력",
                    "notes" => "메모장",
                    _ => "",
                })
                .decorations(false)
                .inner_size(if geo.w > 0.0 { geo.w } else { 408.0 }, if geo.h > 0.0 { geo.h } else { 449.0 })
                .position(geo.x, geo.y)
                .visible(visible)
                .min_inner_size(260.0, 180.0)
                .build()?;
            }

            // 사용법/새 기능 안내 창 — 숨긴 채로 만들고, 보여줄 내용이 있으면 창 스스로 띄운다
            WebviewWindowBuilder::new(app, "guide", WebviewUrl::App("guide.html".into()))
                .title("Todi 사용법")
                .decorations(false)
                .inner_size(560.0, 680.0)
                .min_inner_size(380.0, 320.0)
                .center()
                .visible(false)
                .build()?;

            // 고정해 둔 상세 메모 창 다시 띄우기 (그 사이 지워진 메모는 건너뜀)
            {
                let data = app.state::<Store>().data.lock().unwrap().clone();
                for p in &settings.pinned_notes {
                    let exists = data.memos.iter().any(|m| m.id == p.memo_id && m.notes.iter().any(|n| n.id == p.note_id));
                    if exists {
                        let _ = build_note_window(app.handle(), p.memo_id, p.note_id, &p.geo);
                    }
                }
            }

            // 투명도 적용
            if settings.opacity < 1.0 {
                apply_window_opacity(app.handle(), settings.opacity);
            }

            // 시스템 트레이
            use tauri::menu::MenuBuilder;
            use tauri::tray::TrayIconBuilder;

            let menu = MenuBuilder::new(app)
                .text("show_panel", "패널 열기")
                .text("show_todo", "Todo 보기")
                .text("show_doing", "Doing 보기")
                .text("show_done", "완료 목록")
                .text("show_trash", "휴지통")
                .text("show_calendar", "달력")
                .text("show_notes", "메모장")
                .separator()
                .text("show_guide", "사용법 보기")
                .separator()
                .text("quit", "종료")
                .build()?;

            let icon = app.default_window_icon().cloned()
                .expect("앱 아이콘이 필요합니다");

            TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .tooltip("Todi")
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "quit" => save_and_exit(app),
                        "show_guide" => show_full_guide(app),
                        id => {
                            let label = match id {
                                "show_panel" => "panel",
                                "show_todo" => "todo",
                                "show_doing" => "doing",
                                "show_done" => "done",
                                "show_trash" => "trash",
                                "show_calendar" => "calendar",
                                "show_notes" => "notes",
                                _ => return,
                            };
                            if let Some(w) = app.get_webview_window(label) {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // 자동 시작 설정
            let autostart_manager = app.autolaunch();
            if settings.autostart {
                let _ = autostart_manager.enable();
            } else {
                let _ = autostart_manager.disable();
            }

            // 자동 업데이트 체크
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(updater) = app_handle.updater() {
                    if let Ok(Some(update)) = updater.check().await {
                        let _ = update.download_and_install(|_, _| {}, || {}).await;
                        app_handle.restart();
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_all_data, get_categories, get_settings,
            add_memo, edit_memo, move_memo, complete_memo, restore_memo, uncomplete_memo, delete_memo, permanent_delete, empty_trash,
            toggle_collapse, move_memo_position,
            add_category, delete_category, rename_category, move_category, reorder_categories,
            save_user_settings, get_system_fonts, set_autostart, set_close_to_tray, set_opacity, save_calendar_color,
            start_drag, show_window, hide_window, minimize_window, toggle_on_top, close_app, close_window,
            fit_window_height,
            export_data, open_import_file, import_data,
            sync::get_sync_status, sync::sync_new_code, sync::sync_set_code, sync::sync_clear,
            sync::sync_set_cats, sync::sync_set_lan, sync::sync_pick_folder, sync::sync_set_folder_on, sync::sync_now, sync::sync_set_peer,
            lock::lock_status, lock::lock_setup, lock::lock_now, lock::lock_unlock, lock::lock_recover,
            lock::lock_change_password, lock::lock_set_cats, lock::lock_set_mode, lock::lock_disable, lock::lock_touch,
            add_note, edit_note, delete_note, reorder_notes, open_note, take_pending_note,
            get_guide_state, dismiss_guide, open_guide,
            edit_note_body, open_note_window, close_unpinned_note_windows, get_note_pins, toggle_note_pin, close_note_window,
        ])
        .run(tauri::generate_context!())
        .expect("앱 실행 오류");
}

#[cfg(test)]
mod import_tests {
    use super::*;

    fn memo(id: u64, uid: &str, cat: &str, text: &str, updated: i64) -> Memo {
        let mut m: Memo = serde_json::from_value(serde_json::json!({
            "text": text, "category": cat, "status": "todo", "done_at": null,
        })).unwrap();
        m.id = id;
        m.uid = uid.into();
        m.updated = updated;
        m
    }

    fn data(cats: &[&str], memos: Vec<Memo>) -> AppData {
        AppData { categories: cats.iter().map(|c| c.to_string()).collect(), memos, ..AppData::default() }
    }

    /// export_data와 같은 방식으로 파일 내용을 만들고, JSON을 거쳐 다시 읽는다
    fn export(d: &AppData, cats: &[&str]) -> ExportFile {
        let cats: Vec<String> = cats.iter().map(|c| c.to_string()).collect();
        let f = ExportFile {
            app: "Todi".into(), version: "test".into(), exported: String::new(),
            categories: d.categories.iter().filter(|c| cats.contains(c)).cloned().collect(),
            memos: d.memos.iter().filter(|m| m.status != "trash" && cats.contains(&m.category)).cloned().collect(),
        };
        serde_json::from_str(&serde_json::to_string_pretty(&f).unwrap()).unwrap()
    }

    fn cats(v: &[&str]) -> Vec<String> {
        v.iter().map(|c| c.to_string()).collect()
    }

    fn laptop() -> AppData {
        let mut trash = memo(4, "L4", "업무", "버린것", 100);
        trash.status = "trash".into();
        data(&["업무", "개인", "회의"], vec![
            memo(1, "L1", "업무", "보고서", 100),
            memo(2, "L2", "개인", "운동", 100),
            memo(3, "L3", "회의", "회의록", 100),
            trash,
        ])
    }

    #[test]
    fn export_then_merge_into_other_pc() {
        let f = export(&laptop(), &["업무", "개인"]);
        assert_eq!(f.categories, vec!["업무", "개인"]);
        assert_eq!(f.memos.len(), 2, "휴지통·안 고른 카테고리는 빠져야 함");

        let mut pc = data(&["일반"], vec![memo(1, "P1", "일반", "기존", 50)]);
        let r = import_into(&mut pc, f.clone(), &cats(&["업무", "개인"]), false);
        assert_eq!((r.added, r.skipped, r.removed), (2, 0, 0));
        assert_eq!(pc.categories, vec!["일반", "업무", "개인"]);
        assert_eq!(pc.memos.len(), 3);
        assert!(pc.memos.iter().any(|m| m.uid == "L1"), "uid 유지");
        let ids: Vec<u64> = pc.memos.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3], "id 겹치면 안 됨");

        // 같은 파일을 한 번 더 → 전부 건너뜀
        let r = import_into(&mut pc, f, &cats(&["업무", "개인"]), false);
        assert_eq!((r.added, r.skipped), (0, 2));
        assert_eq!(pc.memos.len(), 3);
    }

    #[test]
    fn only_checked_categories_are_imported() {
        let f = export(&laptop(), &["업무", "개인", "회의"]);
        let mut pc = data(&["일반"], vec![]);
        let r = import_into(&mut pc, f, &cats(&["회의"]), false);
        assert_eq!(r.added, 1);
        assert_eq!(pc.categories, vec!["일반", "회의"]);
        assert_eq!(pc.memos[0].text, "회의록");
    }

    #[test]
    fn merge_keeps_newer_version() {
        let mut lap = laptop();
        let f_old = export(&lap, &["업무"]);
        let mut pc = data(&["업무"], vec![]);
        import_into(&mut pc, f_old.clone(), &cats(&["업무"]), false);
        let pc_id = pc.memos[0].id;

        // 노트북에서 고친 뒤 다시 옮김 → 고친 내용으로
        lap.memos[0].text = "보고서 v2".into();
        lap.memos[0].updated = 200;
        import_into(&mut pc, export(&lap, &["업무"]), &cats(&["업무"]), false);
        assert_eq!(pc.memos.len(), 1);
        assert_eq!(pc.memos[0].text, "보고서 v2");
        assert_eq!(pc.memos[0].id, pc_id, "이 PC의 id는 유지");

        // 예전 파일을 다시 넣어도 최신 내용이 덮어써지지 않음
        let r = import_into(&mut pc, f_old, &cats(&["업무"]), false);
        assert_eq!((r.added, r.skipped), (0, 1));
        assert_eq!(pc.memos[0].text, "보고서 v2");
    }

    #[test]
    fn old_files_without_uid_dedup_by_content() {
        let mut f = export(&laptop(), &["업무"]);
        for m in f.memos.iter_mut() {
            m.uid.clear();
            m.updated = 0;
        }
        let mut pc = data(&["업무"], vec![memo(1, "P1", "업무", "보고서", 50)]);
        let r = import_into(&mut pc, f, &cats(&["업무"]), false);
        assert_eq!((r.added, r.skipped), (0, 1));
    }

    #[test]
    fn replace_mode() {
        let f = export(&laptop(), &["업무"]);
        let mut pc = data(&["업무", "기타"], vec![
            memo(1, "L1", "업무", "보고서 옛날", 50),
            memo(2, "P2", "업무", "PC에만 있던 것", 50),
            memo(3, "P3", "기타", "다른 카테고리", 50),
        ]);
        let r = import_into(&mut pc, f, &cats(&["업무"]), true);
        assert_eq!((r.added, r.removed), (1, 2));
        let mut t: Vec<&str> = pc.memos.iter().map(|m| m.text.as_str()).collect();
        t.sort();
        assert_eq!(t, vec!["다른 카테고리", "보고서"]);
        assert!(pc.deleted.contains_key("P2"), "사라진 항목은 동기화로도 지워지게 기록");
        assert!(!pc.deleted.contains_key("L1"), "다시 넣은 항목은 삭제 기록 없음");
        assert_eq!(pc.memos.iter().find(|m| m.uid == "L1").unwrap().id, 1, "같은 항목은 id 유지");
    }

    #[test]
    fn new_ids_skip_locked_items() {
        let f = export(&laptop(), &["업무"]);
        let mut pc = data(&["업무"], vec![]);
        pc.hidden_max_id = 10;
        import_into(&mut pc, f, &cats(&["업무"]), false);
        assert_eq!(pc.memos[0].id, 11);
    }

    #[test]
    fn memos_json_can_be_picked_directly() {
        let mut d = laptop();
        d.vault = Some("abcd".into());
        let j = serde_json::to_string(&d).unwrap();
        let f: ExportFile = serde_json::from_str(&j).unwrap();
        assert_eq!(f.memos.len(), 4);
    }
}

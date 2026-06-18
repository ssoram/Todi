use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartManagerExt};
use tauri_plugin_updater::UpdaterExt;

// ════════ 데이터 타입 ════════

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Memo {
    text: String,
    #[serde(default)]
    description: String,
    category: String,
    status: String,
    #[serde(default)]
    done: bool,
    done_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AppData {
    categories: Vec<String>,
    memos: Vec<Memo>,
    #[serde(default)]
    collapsed: HashMap<String, bool>,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            categories: vec!["일반".into(), "개발".into(), "회의".into(), "기타".into()],
            memos: vec![],
            collapsed: HashMap::new(),
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
    autostart: bool,
    #[serde(default)]
    recent_fonts: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: "맑은 고딕".into(),
            font_size: 14,
            panel: WinGeo { x: 120.0, y: 80.0, w: 540.0, h: 280.0 },
            todo: WinGeo { x: 120.0, y: 400.0, w: 460.0, h: 540.0 },
            doing: WinGeo { x: 600.0, y: 400.0, w: 460.0, h: 540.0 },
            done: WinGeo { x: 340.0, y: 200.0, w: 420.0, h: 480.0 },
            trash: WinGeo { x: 240.0, y: 150.0, w: 400.0, h: 420.0 },
            autostart: true,
            recent_fonts: vec![],
        }
    }
}

// ════════ 상태 저장소 ════════

struct Store {
    data: Mutex<AppData>,
    settings: Mutex<Settings>,
    data_path: PathBuf,
    settings_path: PathBuf,
}

impl Store {
    fn new(dir: PathBuf) -> Self {
        let dp = dir.join("memos.json");
        let sp = dir.join("settings.json");
        let data: AppData = fs::read_to_string(&dp)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let settings: Settings = fs::read_to_string(&sp)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            data: Mutex::new(data),
            settings: Mutex::new(settings),
            data_path: dp,
            settings_path: sp,
        }
    }

    fn save_data(&self) {
        if let Ok(d) = self.data.lock() {
            if let Ok(j) = serde_json::to_string_pretty(&*d) {
                let _ = fs::write(&self.data_path, j);
            }
        }
    }

    fn save_settings(&self) {
        if let Ok(s) = self.settings.lock() {
            if let Ok(j) = serde_json::to_string_pretty(&*s) {
                let _ = fs::write(&self.settings_path, j);
            }
        }
    }
}

/// 데이터 변경 후 모든 창에 이벤트 전송 + 파일 저장
fn emit_refresh(app: &AppHandle, store: &State<Store>) {
    store.save_data();
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
fn add_memo(app: AppHandle, store: State<Store>, text: String, desc: String, category: String, status: String) {
    store.data.lock().unwrap().memos.push(Memo {
        text, description: desc, category, status,
        done: false, done_at: None,
    });
    emit_refresh(&app, &store);
}

#[tauri::command]
fn edit_memo(app: AppHandle, store: State<Store>, idx: usize, text: String, desc: String, category: Option<String>) {
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.text = text;
        m.description = desc;
        if let Some(cat) = category {
            m.category = cat;
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
        m.done = false;
        m.status = "todo".into();
        m.done_at = None;
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn delete_memo(app: AppHandle, store: State<Store>, idx: usize) {
    // 휴지통으로 이동 (소프트 삭제)
    if let Some(m) = store.data.lock().unwrap().memos.get_mut(idx) {
        m.status = "trash".into();
    }
    emit_refresh(&app, &store);
}

#[tauri::command]
fn permanent_delete(app: AppHandle, store: State<Store>, idx: usize) {
    let mut d = store.data.lock().unwrap();
    if idx < d.memos.len() { d.memos.remove(idx); }
    drop(d);
    emit_refresh(&app, &store);
}

#[tauri::command]
fn empty_trash(app: AppHandle, store: State<Store>) {
    let mut d = store.data.lock().unwrap();
    d.memos.retain(|m| m.status != "trash");
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

// ════════ 카테고리 명령 ════════

#[tauri::command]
fn add_category(app: AppHandle, store: State<Store>, name: String) -> bool {
    let name = name.trim().to_string();
    if name.is_empty() { return false; }
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
    emit_refresh(&app, &store);
}

#[tauri::command]
fn rename_category(app: AppHandle, store: State<Store>, old: String, new_name: String) {
    let nn = new_name.trim().to_string();
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
    for label in ["panel", "todo", "doing", "done", "trash"] {
        if let Some(w) = app.get_webview_window(label) {
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            // 데이터 디렉토리
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;

            let store = Store::new(data_dir);
            let settings = store.settings.lock().unwrap().clone();
            app.manage(store);

            // 창 생성: (label, html, geo, visible)
            let wins: Vec<(&str, &str, &WinGeo, bool)> = vec![
                ("panel", "index.html",  &settings.panel, true),
                ("todo",  "todo.html",   &settings.todo,  true),
                ("doing", "doing.html",  &settings.doing, true),
                ("done",  "done.html",   &settings.done,  false),
                ("trash", "trash.html",  &settings.trash,  false),
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
                    _ => "",
                })
                .decorations(false)
                .inner_size(geo.w, geo.h)
                .position(geo.x, geo.y)
                .visible(visible)
                .min_inner_size(260.0, 180.0)
                .build()?;
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
                        id => {
                            let label = match id {
                                "show_panel" => "panel",
                                "show_todo" => "todo",
                                "show_doing" => "doing",
                                "show_done" => "done",
                        "show_trash" => "trash",
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
            add_memo, edit_memo, move_memo, complete_memo, restore_memo, delete_memo, permanent_delete, empty_trash,
            toggle_collapse,
            add_category, delete_category, rename_category, move_category, reorder_categories,
            save_user_settings, get_system_fonts, set_autostart,
            start_drag, show_window, hide_window, minimize_window, toggle_on_top, close_app,
        ])
        .run(tauri::generate_context!())
        .expect("앱 실행 오류");
}

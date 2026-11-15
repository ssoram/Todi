// ════════ 잠금 (카테고리 숨기기) ════════
//
// 고른 카테고리를 비밀번호로 잠근다. 잠기면 그 카테고리의 항목은 모든 창에서 사라지고,
// memos.json 안에서도 암호화된 덩어리(vault)로만 남는다. 카테고리 이름도 글자로 남기지 않는다.
//
// 키 구조:
// - 항목은 무작위 "데이터 키"(DK)로 암호화한다.
// - DK를 비밀번호에서 만든 키로 한 번(wrap_pw), 복구 코드에서 만든 키로 한 번(wrap_rc) 감싸 둔다.
//   → 비밀번호를 잊어도 복구 코드로 DK를 꺼낼 수 있고, 비밀번호만 바꿀 때 항목을 다시 암호화할 필요가 없다.
//
// 두 방식:
// - always(항상 잠금): 파일은 늘 암호화. 켤 때마다 잠긴 채로 시작하고, 한동안 손대지 않으면 다시 잠긴다.
// - manual(필요할 때만): 평소엔 일반 파일. "잠그기"를 누를 때만 암호화하고, 풀면 다시 일반 파일로 돌아온다.
//   풀려 있는 동안은 DK를 open_key에 둔다 (이때는 내용 자체가 일반 파일이라 더 약해지지 않는다).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::crypto::{from_hex, key_from_password, new_code, normalize_code, open, random_bytes, seal, sha256, to_hex};
use crate::{build_note_window, note_label, sync, AppData, Memo, Settings, Store};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct LockCfg {
    /// "always" | "manual"
    mode: String,
    /// always 모드에서 이 시간(분) 동안 손대지 않으면 다시 잠금. 0 = 안 함
    #[serde(default)]
    idle_min: u32,
    salt: String,
    wrap_pw: String,
    wrap_rc: String,
    /// 잠근 카테고리 이름의 지문 (이름 자체는 저장하지 않음)
    cat_hashes: Vec<String>,
    /// manual 모드에서 풀려 있는 동안의 데이터 키
    #[serde(default)]
    open_key: String,
}

impl LockCfg {
    fn is_locked_cat(&self, name: &str) -> bool {
        self.cat_hashes.contains(&cat_hash(&self.salt, name))
    }
    /// 파일에 쓸 때 잠근 카테고리를 암호화해야 하는지
    fn encrypt_on_disk(&self) -> bool {
        self.mode == "always" || self.open_key.is_empty()
    }
}

/// 암호화되는 내용
#[derive(Serialize, Deserialize, Default)]
struct Vault {
    memos: Vec<Memo>,
    /// (카테고리 이름, 원래 순서)
    categories: Vec<(String, usize)>,
    collapsed: HashMap<String, bool>,
    sync_cats: Vec<String>,
    #[serde(default)]
    sync_off: Vec<String>,
}

fn cat_hash(salt: &str, name: &str) -> String {
    to_hex(&sha256(&[b"todi-lock-cat:", salt.as_bytes(), b":", name.as_bytes()])[..16])
}

/// collapsed 키는 "{상태}:{카테고리}"
fn collapsed_cat(key: &str) -> &str {
    key.split_once(':').map(|(_, c)| c).unwrap_or(key)
}

/// 지금 잠겨 있어서 손댈 수 없는 카테고리인지 알려 주는 함수 (동기화·불러오기·카테고리 추가에서 사용)
pub(crate) fn hidden_filter(s: &Settings, unlocked: bool) -> impl Fn(&str) -> bool {
    let lock = if unlocked { None } else { s.lock.clone() };
    move |name: &str| lock.as_ref().is_some_and(|l| l.is_locked_cat(name))
}

pub(crate) fn is_hidden(store: &Store, name: &str) -> bool {
    let s = store.settings.lock().unwrap();
    hidden_filter(&s, store.lock_key.lock().unwrap().is_some())(name)
}

/// 잠근 카테고리를 떼어 내 암호화한 데이터를 만든다
fn seal_locked(d: &AppData, s: &Settings, l: &LockCfg, key: &[u8; 32]) -> AppData {
    let (locked, plain): (Vec<Memo>, Vec<Memo>) = d.memos.iter().cloned().partition(|m| l.is_locked_cat(&m.category));
    let vault = Vault {
        categories: d.categories.iter().enumerate()
            .filter(|(_, c)| l.is_locked_cat(c)).map(|(i, c)| (c.clone(), i)).collect(),
        collapsed: d.collapsed.iter().filter(|(k, _)| l.is_locked_cat(collapsed_cat(k)))
            .map(|(k, v)| (k.clone(), *v)).collect(),
        sync_cats: s.sync_cats.iter().filter(|c| l.is_locked_cat(c)).cloned().collect(),
        sync_off: s.sync_off.iter().filter(|c| l.is_locked_cat(c)).cloned().collect(),
        memos: locked,
    };
    let hidden_max = vault.memos.iter().map(|m| m.id).max().unwrap_or(0);
    let hidden_ids = vault.memos.iter().map(|m| m.id).collect();
    AppData {
        memos: plain,
        categories: d.categories.iter().filter(|c| !l.is_locked_cat(c)).cloned().collect(),
        collapsed: d.collapsed.iter().filter(|(k, _)| !l.is_locked_cat(collapsed_cat(k)))
            .map(|(k, v)| (k.clone(), *v)).collect(),
        deleted: d.deleted.clone(),
        vault: Some(to_hex(&seal(key, &serde_json::to_vec(&vault).unwrap_or_default()))),
        hidden_max_id: d.hidden_max_id.max(hidden_max),
        hidden_ids,
    }
}

/// 파일에 쓸 모양. 풀려 있는 always 모드면 잠근 카테고리를 그때그때 암호화해서 쓴다.
pub(crate) fn for_disk(d: &AppData, s: &Settings, key: Option<[u8; 32]>) -> AppData {
    match (&s.lock, key) {
        (Some(l), Some(k)) if l.encrypt_on_disk() => seal_locked(d, s, l, &k),
        _ => d.clone(),
    }
}

/// settings.json에 쓸 모양: 잠근 카테고리 이름이 동기화 목록에 글자로 남지 않게
pub(crate) fn settings_for_disk(s: &Settings) -> Settings {
    let mut out = s.clone();
    if let Some(l) = s.lock.as_ref().filter(|l| l.encrypt_on_disk()) {
        out.sync_cats.retain(|c| !l.is_locked_cat(c));
        out.sync_off.retain(|c| !l.is_locked_cat(c));
    }
    out
}

/// 암호화된 덩어리를 풀어서 데이터에 되돌린다
fn apply_vault(d: &mut AppData, s: &mut Settings, key: &[u8; 32]) -> Result<(), String> {
    let vault: Vault = match &d.vault {
        Some(v) => {
            let bytes = from_hex(v).ok_or("잠긴 데이터가 손상됐어요.")?;
            let plain = open(key, &bytes).ok_or("잠긴 데이터를 풀지 못했어요.")?;
            serde_json::from_slice(&plain).map_err(|_| "잠긴 데이터를 읽지 못했어요.")?
        }
        None => Vault::default(),
    };
    let mut cats = vault.categories;
    cats.sort_by_key(|(_, i)| *i);
    for (c, i) in cats {
        if !d.categories.contains(&c) {
            let at = i.min(d.categories.len());
            d.categories.insert(at, c);
        }
    }
    for m in vault.memos {
        // 잠긴 동안 다른 기기에서 지운 항목
        if d.deleted.get(&m.uid).is_some_and(|&t| t >= m.updated) || d.memos.iter().any(|x| x.uid == m.uid) {
            continue;
        }
        d.memos.push(m);
    }
    d.collapsed.extend(vault.collapsed);
    for c in vault.sync_cats {
        if !s.sync_cats.contains(&c) {
            s.sync_cats.push(c);
        }
    }
    for c in vault.sync_off {
        if !s.sync_off.contains(&c) {
            s.sync_off.push(c);
        }
    }
    d.vault = None;
    d.hidden_ids.clear();
    Ok(())
}

/// 앱을 켤 때: manual 모드에서 풀려 있던 상태면 그대로 풀어 둔다
pub(crate) fn on_load(d: &mut AppData, s: &mut Settings) -> Option<[u8; 32]> {
    let l = s.lock.clone()?;
    if l.mode != "manual" || l.open_key.is_empty() {
        return None;
    }
    let key: [u8; 32] = from_hex(&l.open_key)?.try_into().ok()?;
    if d.vault.is_some() && apply_vault(d, s, &key).is_err() {
        return None;
    }
    Some(key)
}

fn unwrap_key(wrapped: &str, kek: &[u8; 32]) -> Option<[u8; 32]> {
    open(kek, &from_hex(wrapped)?)?.try_into().ok()
}

fn salt_of(l: &LockCfg) -> Vec<u8> {
    from_hex(&l.salt).unwrap_or_default()
}

fn notify(app: &AppHandle) {
    let _ = app.emit("lock-changed", ());
}

fn emit_data(app: &AppHandle, store: &Store) {
    let d = store.data.lock().unwrap().clone();
    let _ = app.emit("data-changed", &d);
}

/// 지금 잠그기
pub(crate) fn lock_now_inner(app: &AppHandle) {
    let store = app.state::<Store>();
    {
        let mut d = store.data.lock().unwrap();
        let mut s = store.settings.lock().unwrap();
        let mut key = store.lock_key.lock().unwrap();
        let Some(k) = *key else { return };
        if !lock_data(&mut d, &mut s, &k) { return }
        *key = None;
    }
    store.save_data();
    store.save_settings();
    emit_data(app, &store);
    notify(app);
}

/// 잠그기의 실제 처리: 잠근 카테고리를 떼어 내 암호화하고, 풀린 상태의 키를 지운다
fn lock_data(d: &mut AppData, s: &mut Settings, k: &[u8; 32]) -> bool {
    let Some(l) = s.lock.clone() else { return false };
    *d = seal_locked(d, s, &l, k);
    s.sync_cats.retain(|c| !l.is_locked_cat(c));
    s.sync_off.retain(|c| !l.is_locked_cat(c));
    if let Some(l) = s.lock.as_mut() {
        l.open_key.clear();
    }
    true
}

/// 풀기의 실제 처리: 암호화된 내용을 되돌리고, manual 모드면 풀린 상태를 기억한다
fn unlock_data(d: &mut AppData, s: &mut Settings, key: &[u8; 32]) -> Result<(), String> {
    apply_vault(d, s, key)?;
    if let Some(l) = s.lock.as_mut().filter(|l| l.mode == "manual") {
        l.open_key = to_hex(key);
    }
    Ok(())
}

/// DK를 얻은 뒤 잠금 풀기
fn unlock_with(app: &AppHandle, key: [u8; 32]) -> Result<(), String> {
    let store = app.state::<Store>();
    {
        let mut d = store.data.lock().unwrap();
        let mut s = store.settings.lock().unwrap();
        let mut cur = store.lock_key.lock().unwrap();
        if cur.is_none() {
            unlock_data(&mut d, &mut s, &key)?;
            *cur = Some(key);
        } else if let Some(l) = s.lock.as_mut().filter(|l| l.mode == "manual") {
            l.open_key = to_hex(&key);
        }
    }
    *store.last_active.lock().unwrap() = Instant::now();
    store.save_data();
    store.save_settings();
    emit_data(app, &store);
    reopen_pinned_notes(app);
    sync::resync(app);
    notify(app);
    Ok(())
}

/// 잠겨서 닫혔던 고정 메모 창 다시 띄우기
fn reopen_pinned_notes(app: &AppHandle) {
    let store = app.state::<Store>();
    let pins = store.settings.lock().unwrap().pinned_notes.clone();
    let d = store.data.lock().unwrap().clone();
    for p in pins {
        let exists = d.memos.iter().any(|m| m.id == p.memo_id && m.notes.iter().any(|n| n.id == p.note_id));
        if exists && app.get_webview_window(&note_label(p.memo_id, p.note_id)).is_none() {
            let _ = build_note_window(app, p.memo_id, p.note_id, &p.geo);
        }
    }
}

/// always 모드: 한동안 손대지 않으면 다시 잠금
pub(crate) fn start_idle_watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        let store = app.state::<Store>();
        let idle = {
            let s = store.settings.lock().unwrap();
            match &s.lock {
                Some(l) if l.mode == "always" && l.idle_min > 0 => Some(Duration::from_secs(l.idle_min as u64 * 60)),
                _ => None,
            }
        };
        let Some(idle) = idle else { continue };
        let unlocked = store.lock_key.lock().unwrap().is_some();
        if unlocked && store.last_active.lock().unwrap().elapsed() > idle {
            lock_now_inner(&app);
        }
    });
}

// ──────── 화면 명령 ────────

#[derive(Serialize)]
pub(crate) struct LockStatus {
    configured: bool,
    locked: bool,
    mode: String,
    idle_min: u32,
    /// 잠근 카테고리 (풀려 있을 때만)
    cats: Vec<String>,
}

#[tauri::command]
pub(crate) fn lock_status(app: AppHandle) -> LockStatus {
    let store = app.state::<Store>();
    let d = store.data.lock().unwrap();
    let s = store.settings.lock().unwrap();
    let unlocked = store.lock_key.lock().unwrap().is_some();
    match &s.lock {
        None => LockStatus { configured: false, locked: false, mode: String::new(), idle_min: 0, cats: vec![] },
        Some(l) => LockStatus {
            configured: true,
            locked: !unlocked,
            mode: l.mode.clone(),
            idle_min: l.idle_min,
            cats: if unlocked { d.categories.iter().filter(|c| l.is_locked_cat(c)).cloned().collect() } else { vec![] },
        },
    }
}

fn check_password(pw: &str) -> Result<(), String> {
    if pw.chars().count() < 4 {
        return Err("비밀번호는 4자 이상으로 정해 주세요.".into());
    }
    Ok(())
}

fn check_mode(mode: &str) -> Result<(), String> {
    if mode != "always" && mode != "manual" {
        return Err("잠금 방식이 올바르지 않아요.".into());
    }
    Ok(())
}

/// 새 잠금 설정과 데이터 키, 복구 코드를 만든다
fn new_cfg(password: &str, cats: &[String], mode: &str, idle_min: u32) -> (LockCfg, [u8; 32], String) {
    let dk: [u8; 32] = random_bytes();
    let salt: [u8; 16] = random_bytes();
    let salt_hex = to_hex(&salt);
    let rc = new_code();
    let cfg = LockCfg {
        cat_hashes: cats.iter().map(|c| cat_hash(&salt_hex, c)).collect(),
        mode: mode.to_string(),
        idle_min,
        salt: salt_hex,
        wrap_pw: to_hex(&seal(&key_from_password(password, &salt), &dk)),
        wrap_rc: to_hex(&seal(&key_from_password(&rc, &salt), &dk)),
        open_key: if mode == "manual" { to_hex(&dk) } else { String::new() },
    };
    (cfg, dk, rc)
}

/// 잠금 설정. 복구 코드를 돌려준다 (이때 한 번만 보여 줌).
#[tauri::command]
pub(crate) async fn lock_setup(app: AppHandle, password: String, cats: Vec<String>, mode: String, idle_min: u32) -> Result<String, String> {
    check_password(&password)?;
    check_mode(&mode)?;
    if cats.is_empty() {
        return Err("숨길 카테고리를 하나 이상 골라 주세요.".into());
    }
    let store = app.state::<Store>();
    if store.settings.lock().unwrap().lock.is_some() {
        return Err("이미 잠금이 설정돼 있어요.".into());
    }
    let (cfg, dk, rc) = new_cfg(&password, &cats, &mode, idle_min);
    {
        let mut s = store.settings.lock().unwrap();
        s.lock = Some(cfg);
        *store.lock_key.lock().unwrap() = Some(dk);
    }
    *store.last_active.lock().unwrap() = Instant::now();
    store.save_data();
    store.save_settings();
    notify(&app);
    Ok(rc)
}

#[tauri::command]
pub(crate) fn lock_now(app: AppHandle) {
    lock_now_inner(&app);
}

#[tauri::command]
pub(crate) async fn lock_unlock(app: AppHandle, password: String) -> Result<(), String> {
    let l = app.state::<Store>().settings.lock().unwrap().lock.clone().ok_or("잠금이 설정돼 있지 않아요.")?;
    let kek = key_from_password(&password, &salt_of(&l));
    let Some(dk) = unwrap_key(&l.wrap_pw, &kek) else {
        // 비밀번호를 연달아 넣어 보는 걸 느리게
        std::thread::sleep(Duration::from_millis(800));
        return Err("비밀번호가 맞지 않아요.".into());
    };
    unlock_with(&app, dk)
}

/// 비밀번호를 잊었을 때: 복구 코드로 풀고 새 비밀번호를 정한다
#[tauri::command]
pub(crate) async fn lock_recover(app: AppHandle, code: String, new_password: String) -> Result<(), String> {
    check_password(&new_password)?;
    let code = normalize_code(&code).ok_or("복구 코드 형식이 맞지 않아요. 16자리 코드를 그대로 넣어 주세요.")?;
    let store = app.state::<Store>();
    let l = store.settings.lock().unwrap().lock.clone().ok_or("잠금이 설정돼 있지 않아요.")?;
    let salt = salt_of(&l);
    let Some(dk) = unwrap_key(&l.wrap_rc, &key_from_password(&code, &salt)) else {
        std::thread::sleep(Duration::from_millis(800));
        return Err("복구 코드가 맞지 않아요.".into());
    };
    let wrap_pw = to_hex(&seal(&key_from_password(&new_password, &salt), &dk));
    if let Some(l) = store.settings.lock().unwrap().lock.as_mut() {
        l.wrap_pw = wrap_pw;
    }
    unlock_with(&app, dk)
}

fn unlocked_key(store: &Store) -> Result<[u8; 32], String> {
    store.lock_key.lock().unwrap().ok_or_else(|| "먼저 잠금을 풀어 주세요.".to_string())
}

#[tauri::command]
pub(crate) async fn lock_change_password(app: AppHandle, new_password: String) -> Result<(), String> {
    check_password(&new_password)?;
    let store = app.state::<Store>();
    let dk = unlocked_key(&store)?;
    let salt = store.settings.lock().unwrap().lock.as_ref().map(salt_of).ok_or("잠금이 설정돼 있지 않아요.")?;
    let wrap_pw = to_hex(&seal(&key_from_password(&new_password, &salt), &dk));
    if let Some(l) = store.settings.lock().unwrap().lock.as_mut() {
        l.wrap_pw = wrap_pw;
    }
    store.save_settings();
    Ok(())
}

#[tauri::command]
pub(crate) fn lock_set_cats(app: AppHandle, cats: Vec<String>) -> Result<(), String> {
    if cats.is_empty() {
        return Err("숨길 카테고리를 하나 이상 골라 주세요. 아무것도 숨기지 않으려면 잠금 끄기를 눌러 주세요.".into());
    }
    let store = app.state::<Store>();
    unlocked_key(&store)?;
    if let Some(l) = store.settings.lock().unwrap().lock.as_mut() {
        l.cat_hashes = cats.iter().map(|c| cat_hash(&l.salt, c)).collect();
    }
    store.save_data();
    store.save_settings();
    notify(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn lock_set_mode(app: AppHandle, mode: String, idle_min: u32) -> Result<(), String> {
    check_mode(&mode)?;
    let store = app.state::<Store>();
    let dk = unlocked_key(&store)?;
    if let Some(l) = store.settings.lock().unwrap().lock.as_mut() {
        l.open_key = if mode == "manual" { to_hex(&dk) } else { String::new() };
        l.mode = mode;
        l.idle_min = idle_min;
    }
    *store.last_active.lock().unwrap() = Instant::now();
    store.save_data();
    store.save_settings();
    notify(&app);
    Ok(())
}

/// 잠금 끄기: 숨긴 카테고리를 다시 일반 데이터로 돌려 놓는다
#[tauri::command]
pub(crate) fn lock_disable(app: AppHandle) -> Result<(), String> {
    let store = app.state::<Store>();
    unlocked_key(&store)?;
    store.settings.lock().unwrap().lock = None;
    *store.lock_key.lock().unwrap() = None;
    store.save_data();
    store.save_settings();
    notify(&app);
    Ok(())
}

/// 창에서 마우스·키보드를 쓸 때마다 (다시 잠금 시간 계산용)
#[tauri::command]
pub(crate) fn lock_touch(app: AppHandle) {
    *app.state::<Store>().last_active.lock().unwrap() = Instant::now();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(id: u64, cat: &str) -> Memo {
        let mut m: Memo = serde_json::from_value(serde_json::json!({
            "text": format!("비밀{id}"), "category": cat, "status": "todo", "done_at": null,
        })).unwrap();
        m.id = id;
        m.uid = format!("u{id}");
        m
    }

    fn setup() -> (AppData, Settings, LockCfg, [u8; 32]) {
        let mut d = AppData::default();
        d.categories = vec!["일반".into(), "개인".into(), "회의".into()];
        d.memos = vec![memo(1, "일반"), memo(2, "개인"), memo(3, "회의")];
        d.collapsed.insert("todo:개인".into(), true);
        let mut s = Settings::default();
        s.sync_cats = vec!["일반".into(), "개인".into()];
        let salt = "00ff".to_string();
        let l = LockCfg {
            mode: "always".into(), idle_min: 0, salt: salt.clone(), wrap_pw: String::new(), wrap_rc: String::new(),
            cat_hashes: vec![cat_hash(&salt, "개인")], open_key: String::new(),
        };
        s.lock = Some(l.clone());
        (d, s, l, random_bytes())
    }

    #[test]
    fn sealed_file_has_no_secret_text_and_roundtrips() {
        let (d, s, l, k) = setup();
        let sealed = seal_locked(&d, &s, &l, &k);
        let json = serde_json::to_string(&sealed).unwrap();
        assert!(!json.contains("개인"), "카테고리 이름이 남으면 안 됨");
        assert!(!json.contains("비밀2"), "잠근 항목 내용이 남으면 안 됨");
        assert!(json.contains("비밀1"));
        assert_eq!(sealed.hidden_ids, vec![2]);
        assert_eq!(sealed.hidden_max_id, 2);

        let mut back = sealed;
        let mut s2 = s.clone();
        s2.sync_cats = vec!["일반".into()];
        apply_vault(&mut back, &mut s2, &k).unwrap();
        assert_eq!(back.categories, vec!["일반", "개인", "회의"]);
        assert!(back.memos.iter().any(|m| m.text == "비밀2"));
        assert_eq!(back.collapsed.get("todo:개인"), Some(&true));
        assert!(s2.sync_cats.contains(&"개인".to_string()));
        assert!(back.vault.is_none());
    }

    #[test]
    fn wrong_key_cannot_open() {
        let (d, s, l, k) = setup();
        let mut sealed = seal_locked(&d, &s, &l, &k);
        let mut s2 = s.clone();
        assert!(apply_vault(&mut sealed, &mut s2, &random_bytes()).is_err());
    }

    #[test]
    fn settings_file_hides_locked_sync_category() {
        let (_, s, _, _) = setup();
        let out = settings_for_disk(&s);
        assert_eq!(out.sync_cats, vec!["일반"]);
    }

    #[test]
    fn password_and_recovery_both_unwrap() {
        let dk: [u8; 32] = random_bytes();
        let salt: [u8; 16] = random_bytes();
        let rc = new_code();
        let wp = to_hex(&seal(&key_from_password("pass1234", &salt), &dk));
        let wr = to_hex(&seal(&key_from_password(&rc, &salt), &dk));
        assert_eq!(unwrap_key(&wp, &key_from_password("pass1234", &salt)), Some(dk));
        assert_eq!(unwrap_key(&wr, &key_from_password(&normalize_code(&rc.to_lowercase()).unwrap(), &salt)), Some(dk));
        assert_eq!(unwrap_key(&wp, &key_from_password("wrong", &salt)), None);
    }

    #[test]
    fn hidden_filter_only_when_locked() {
        let (_, s, _, _) = setup();
        assert!(hidden_filter(&s, false)("개인"));
        assert!(!hidden_filter(&s, false)("일반"));
        assert!(!hidden_filter(&s, true)("개인"));
    }
}

/// 실제 파일 저장 → 앱 재시작 → 풀기 흐름을 그대로 따라가는 테스트
#[cfg(test)]
mod flow_tests {
    use super::*;

    fn memo(id: u64, cat: &str, text: &str) -> Memo {
        let mut m: Memo = serde_json::from_value(serde_json::json!({
            "text": text, "category": cat, "status": "todo", "done_at": null,
        })).unwrap();
        m.id = id;
        m.uid = format!("u{id}");
        m.updated = 1000;
        m
    }

    fn sample() -> (AppData, Settings) {
        let mut d = AppData::default();
        d.categories = vec!["업무".into(), "개인일기".into(), "회의".into()];
        d.memos = vec![memo(1, "업무", "보고서"), memo(2, "개인일기", "연애편지"), memo(3, "회의", "주간회의"), memo(4, "개인일기", "병원예약")];
        let mut s = Settings::default();
        s.sync_cats = vec!["업무".into(), "개인일기".into()];
        (d, s)
    }

    /// 디스크에 쓰고 다시 읽기 (앱 재시작 흉내). 두 파일 내용을 함께 돌려준다.
    fn restart(d: &AppData, s: &Settings, key: Option<[u8; 32]>) -> (AppData, Settings, Option<[u8; 32]>, String) {
        let dj = serde_json::to_string_pretty(&for_disk(d, s, key)).unwrap();
        let sj = serde_json::to_string_pretty(&settings_for_disk(s)).unwrap();
        let mut d2: AppData = serde_json::from_str(&dj).unwrap();
        let mut s2: Settings = serde_json::from_str(&sj).unwrap();
        let k = on_load(&mut d2, &mut s2);
        (d2, s2, k, dj + &sj)
    }

    fn texts(d: &AppData) -> Vec<String> {
        let mut v: Vec<String> = d.memos.iter().map(|m| m.text.clone()).collect();
        v.sort();
        v
    }

    fn pw_key(s: &Settings, pw: &str) -> Option<[u8; 32]> {
        let l = s.lock.as_ref().unwrap();
        unwrap_key(&l.wrap_pw, &key_from_password(pw, &salt_of(l)))
    }

    #[test]
    fn always_mode_full_cycle() {
        let (d, mut s) = sample();
        let orig = texts(&d);
        let (cfg, dk, rc) = new_cfg("1234", &["개인일기".into()], "always", 5);
        s.lock = Some(cfg);

        // 풀린 채로 저장해도 파일에는 비밀이 없어야 함 → 재시작하면 잠긴 상태
        let (mut d2, mut s2, k, files) = restart(&d, &s, Some(dk));
        for secret in ["개인일기", "연애편지", "병원예약", "1234", rc.as_str()] {
            assert!(!files.contains(secret), "파일에 {secret} 가 보임");
        }
        assert!(k.is_none(), "always 모드는 잠긴 채로 시작");
        assert_eq!(texts(&d2), vec!["보고서", "주간회의"]);
        assert_eq!(d2.categories, vec!["업무", "회의"]);
        assert_eq!(s2.sync_cats, vec!["업무"]);

        // 틀린 비밀번호
        assert!(pw_key(&s2, "0000").is_none());
        // 맞는 비밀번호로 풀기
        let key = pw_key(&s2, "1234").unwrap();
        unlock_data(&mut d2, &mut s2, &key).unwrap();
        assert_eq!(texts(&d2), orig);
        assert_eq!(d2.categories, vec!["업무", "개인일기", "회의"]);
        assert!(s2.sync_cats.contains(&"개인일기".to_string()));
        assert!(s2.lock.as_ref().unwrap().open_key.is_empty(), "always 모드는 키를 파일에 남기지 않음");

        // 복구 코드로도 (소문자·하이픈 섞어 넣어도) 풀림
        let (mut d3, mut s3, _, _) = restart(&d2, &s2, Some(key));
        let l = s3.lock.clone().unwrap();
        let typed: String = rc.to_lowercase().chars().enumerate()
            .flat_map(|(i, c)| if i > 0 && i % 4 == 0 { vec!['-', c] } else { vec![c] }).collect();
        let code = normalize_code(&typed).unwrap();
        let key2 = unwrap_key(&l.wrap_rc, &key_from_password(&code, &salt_of(&l))).unwrap();
        unlock_data(&mut d3, &mut s3, &key2).unwrap();
        assert_eq!(texts(&d3), orig);
    }

    #[test]
    fn manual_mode_plain_until_locked() {
        let (d, mut s) = sample();
        let (cfg, dk, _) = new_cfg("abcd", &["개인일기".into()], "manual", 0);
        s.lock = Some(cfg);

        // 풀린 상태: 재시작해도 풀린 그대로
        let (mut d2, mut s2, k, _) = restart(&d, &s, Some(dk));
        assert_eq!(k, Some(dk));
        assert_eq!(d2.memos.len(), 4);

        // 잠그기 → 파일에 비밀 없음, 재시작해도 잠김
        assert!(lock_data(&mut d2, &mut s2, &dk));
        assert_eq!(d2.memos.len(), 2);
        let (mut d3, mut s3, k3, files) = restart(&d2, &s2, None);
        assert!(k3.is_none());
        assert!(!files.contains("연애편지") && !files.contains("개인일기"));
        assert!(!files.contains(&to_hex(&dk)), "잠근 뒤엔 키가 파일에 없어야 함");

        // 풀기 → 다시 일반 상태, 재시작해도 풀린 그대로
        let key = pw_key(&s3, "abcd").unwrap();
        unlock_data(&mut d3, &mut s3, &key).unwrap();
        let (d4, _, k4, _) = restart(&d3, &s3, Some(key));
        assert_eq!(k4, Some(key));
        assert_eq!(texts(&d4), texts(&d));
    }

    #[test]
    fn changes_while_locked_are_kept() {
        let (d, mut s) = sample();
        let (cfg, dk, _) = new_cfg("1234", &["개인일기".into()], "always", 0);
        s.lock = Some(cfg);
        let (mut d2, s2, _, _) = restart(&d, &s, Some(dk));

        // 잠긴 동안 새 항목 추가: 숨긴 항목과 id가 겹치면 안 됨
        let id = crate::max_id(&d2) + 1;
        assert!(id > 4, "잠긴 항목 id(4)와 겹침: {id}");
        d2.memos.push(memo(id, "업무", "새일"));
        // 잠긴 동안 다른 기기에서 숨긴 항목(u4)을 지움
        d2.deleted.insert("u4".into(), 2000);
        // 저장 → 재시작 → 풀기
        let (mut d3, mut s3, _, _) = restart(&d2, &s2, None);
        let key = pw_key(&s3, "1234").unwrap();
        unlock_data(&mut d3, &mut s3, &key).unwrap();
        assert_eq!(texts(&d3), vec!["보고서", "새일", "연애편지", "주간회의"]);
        let mut ids: Vec<u64> = d3.memos.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4, "id가 겹침");
        assert!(d3.hidden_ids.is_empty() && d3.vault.is_none());
    }

    #[test]
    fn changing_locked_categories_and_disabling() {
        let (d, mut s) = sample();
        let (cfg, dk, _) = new_cfg("1234", &["개인일기".into()], "always", 0);
        s.lock = Some(cfg);
        // 숨길 카테고리를 "회의"로 바꿈
        if let Some(l) = s.lock.as_mut() {
            l.cat_hashes = vec![cat_hash(&l.salt, "회의")];
        }
        let (d2, _, _, files) = restart(&d, &s, Some(dk));
        assert!(files.contains("연애편지") && !files.contains("주간회의"));
        assert_eq!(texts(&d2), vec!["병원예약", "보고서", "연애편지"]);
        // 잠금 끄기: 키 없이 저장하면 모두 일반 파일
        s.lock = None;
        let (d3, _, _, files) = restart(&d, &s, None);
        assert!(files.contains("주간회의") && !files.contains("vault"));
        assert_eq!(d3.memos.len(), 4);
    }
}

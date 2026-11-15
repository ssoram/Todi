// ════════ 암호화 공통 (동기화 · 잠금에서 함께 사용) ════════

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

/// 코드에 쓰는 글자 (헷갈리는 0/O, 1/I 제외, 32자 → 글자당 5비트, 16자 = 80비트)
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// 비밀번호 → 키 변환 반복 횟수. 비밀번호를 하나하나 넣어 보는 공격을 느리게 만든다.
const PBKDF2_ROUNDS: u32 = 300_000;

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    let _ = getrandom::getrandom(&mut b);
    b
}

pub(crate) fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub(crate) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// "XXXX-XXXX-XXXX-XXXX" 형태의 무작위 코드 (동기화 코드, 복구 코드)
pub(crate) fn new_code() -> String {
    let b: [u8; 16] = random_bytes();
    let chars: Vec<char> = b.iter().map(|x| ALPHABET[(*x % 32) as usize] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// 사용자가 입력한 코드를 "XXXX-XXXX-XXXX-XXXX" 형태로. 형식이 틀리면 None.
pub(crate) fn normalize_code(input: &str) -> Option<String> {
    let raw: String = input.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if raw.len() != 16 || !raw.bytes().all(|b| ALPHABET.contains(&b)) {
        return None;
    }
    Some(raw.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-"))
}

/// 비밀번호(또는 복구 코드) → 32바이트 키
pub(crate) fn key_from_password(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ROUNDS, &mut out);
    out
}

pub(crate) fn seal(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = random_bytes();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain).unwrap_or_default();
    let mut out = b"TODI1".to_vec();
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// 복호화. 키가 다르거나 내용이 바뀌었으면 None.
pub(crate) fn open(key: &[u8; 32], data: &[u8]) -> Option<Vec<u8>> {
    let rest = data.strip_prefix(b"TODI1")?;
    if rest.len() < 12 {
        return None;
    }
    let (nonce, ct) = rest.split_at(12);
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(Nonce::from_slice(nonce), ct).ok()
}

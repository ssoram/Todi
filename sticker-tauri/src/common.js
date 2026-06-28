// ═══ SVG 아이콘 ═══
const S={
todo:'<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 4h2a2 2 0 012 2v14a2 2 0 01-2 2H6a2 2 0 01-2-2V6a2 2 0 012-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/><line x1="9" y1="10" x2="15" y2="10"/><line x1="9" y1="14" x2="13" y2="14"/></svg>',
doing:'<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/></svg>',
done:'<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M8 12l3 3 5-5"/></svg>',
pin:'<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M16 2L14.5 3.5l1 1-5.5 5.5-3-1L5 11l4 4-5 6 6-5 4 4 2-2-1-3 5.5-5.5 1 1L24 8z"/></svg>',
unpin:'<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><path d="M16 2L14.5 3.5l1 1-5.5 5.5-3-1L5 11l4 4-5 6 6-5 4 4 2-2-1-3 5.5-5.5 1 1L24 8z"/><line x1="2" y1="2" x2="22" y2="22" stroke-width="2.5"/></svg>',
plus:'<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>',
x:'<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="6" y1="6" x2="18" y2="18"/><line x1="18" y1="6" x2="6" y2="18"/></svg>',
min:'<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="5" y1="12" x2="19" y2="12"/></svg>',
right:'<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4l8 8-8 8"/></svg>',
left:'<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M15 4l-8 8 8 8"/></svg>',
trash:'<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 6h18M8 6V4h8v2M5 6v14a2 2 0 002 2h10a2 2 0 002-2V6"/></svg>',
edit:'<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3l4 4L7 21H3v-4z"/></svg>',
undo:'<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 11l-4-4 4-4"/><path d="M3 7h12a5 5 0 010 10h-3"/></svg>',
gear:'<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8"><line x1="4" y1="6" x2="20" y2="6"/><line x1="4" y1="12" x2="20" y2="12"/><line x1="4" y1="18" x2="20" y2="18"/><circle cx="9" cy="6" r="2" fill="currentColor"/><circle cx="16" cy="12" r="2" fill="currentColor"/><circle cx="11" cy="18" r="2" fill="currentColor"/></svg>',
tag:'<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.59 13.41l-7.17 7.17a2 2 0 01-2.83 0L2 12V2h10l8.59 8.59a2 2 0 010 2.82z"/><circle cx="7" cy="7" r="1.5" fill="currentColor"/></svg>',
cal:'<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>',
mic:'<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="1" width="6" height="11" rx="3"/><path d="M19 10v2a7 7 0 01-14 0v-2"/><line x1="12" y1="19" x2="12" y2="23"/><line x1="8" y1="23" x2="16" y2="23"/></svg>',
micOn:'<svg viewBox="0 0 24 24" width="14" height="14" fill="red" stroke="red" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="1" width="6" height="11" rx="3"/><path d="M19 10v2a7 7 0 01-14 0v-2"/><line x1="12" y1="19" x2="12" y2="23"/><line x1="8" y1="23" x2="16" y2="23"/></svg>',
};

// ═══ Tauri API ═══
const {invoke} = window.__TAURI__.core;
const {listen} = window.__TAURI__.event;

function esc(t){const d=document.createElement('div');d.textContent=t||'';return d.innerHTML}

// 설정에서 폰트 적용
async function applyFont(){
  try{
    const s=await invoke('get_settings');
    document.documentElement.style.setProperty('--font',`'${s.font_family}','나눔고딕','맑은 고딕',sans-serif`);
    document.documentElement.style.setProperty('--fs',s.font_size+'px');
  }catch(e){}
}

const CC=['#FFF8E1','#E8F5E9','#F3E5F5','#E0F7FA','#FBE9E7','#F1F8E9','#EDE7F6','#E0F2F1'];

// ═══ 설정 변경 즉시 적용 (모든 창에서 수신) ═══
listen('settings-changed', e=>{
  const s=e.payload;
  document.documentElement.style.setProperty('--font',`'${s.font_family}','나눔고딕','맑은 고딕',sans-serif`);
  document.documentElement.style.setProperty('--fs',s.font_size+'px');
});

// ═══ 위치 고정 상태 ═══
let _posLocked = false;

function isPosLocked(){ return _posLocked; }

function togglePosLock(){
  _posLocked = !_posLocked;
  const hdr = document.querySelector('.hdr');
  if(hdr) hdr.classList.toggle('locked', _posLocked);
  const pinB = document.getElementById('pinB');
  if(pinB) pinB.innerHTML = _posLocked ? S.pin : S.unpin;
  return _posLocked;
}

// ═══ 헤더 호버 + 드래그 (JS 기반) ═══
document.addEventListener('DOMContentLoaded',()=>{
  document.querySelectorAll('.hdr').forEach(hdr=>{
    // 호버
    hdr.addEventListener('mouseenter',()=>hdr.classList.add('hover'));
    hdr.addEventListener('mouseleave',()=>hdr.classList.remove('hover'));
    // 드래그 — 위치 고정이 아닐 때만 창 이동
    hdr.addEventListener('mousedown',e=>{
      if(e.target.closest('button,input,select,textarea,.ct'))return;
      if(_posLocked) return; // 위치 고정이면 이동 불가
      invoke('start_drag');
    });
  });
});

// ═══ 커스텀 확인 팝업 ═══
function customConfirm(msg){
  return new Promise(resolve=>{
    // 기존 팝업 제거
    const old=document.getElementById('_cf');if(old)old.remove();
    const ov=document.createElement('div');
    ov.id='_cf';ov.className='confirm-overlay show';
    ov.innerHTML=`<div class="confirm-box">
      <div class="ci">${S.trash}</div>
      <div class="cm">${esc(msg)}</div>
      <div class="cb">
        <button class="btn-cancel" id="_cfN">취소</button>
        <button class="btn-ok" id="_cfY">삭제</button>
      </div></div>`;
    document.body.appendChild(ov);
    document.getElementById('_cfY').onclick=()=>{ov.remove();resolve(true)};
    document.getElementById('_cfN').onclick=()=>{ov.remove();resolve(false)};
    ov.onclick=e=>{if(e.target===ov){ov.remove();resolve(false)}};
  });
}

// ═══ 음성 인식 ═══
// 마지막 포커스된 input/textarea 추적
let _lastFocused=null;
document.addEventListener('focusin',e=>{
  if(e.target.matches('input[type=text],textarea'))_lastFocused=e.target;
});

function startVoice(btn, fallbackInput){
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  if(!SR){alert('이 브라우저는 음성 인식을 지원하지 않습니다.');return}
  if(btn._recog){btn._recog.stop();btn._recog=null;btn.innerHTML=S.mic;btn.classList.remove('voice-on');return}
  const target=_lastFocused||fallbackInput;
  const r=new SR();
  r.lang='ko-KR';r.continuous=false;r.interimResults=false;
  btn._recog=r;btn.innerHTML=S.micOn;btn.classList.add('voice-on');
  r.onresult=e=>{
    const t=e.results[0][0].transcript;
    if(target){target.value+=t;target.focus()}
  };
  r.onend=()=>{btn._recog=null;btn.innerHTML=S.mic;btn.classList.remove('voice-on')};
  r.onerror=()=>{btn._recog=null;btn.innerHTML=S.mic;btn.classList.remove('voice-on')};
  r.start();
}

// ═══ 카드 드래그 정렬 ═══
function initCardDrag(container){
  container.addEventListener('pointerdown',e=>{
    const handle=e.target.closest('.cd-dh');
    if(!handle)return;
    const cd=handle.closest('.cd');
    if(!cd||!cd.dataset.idx)return;
    e.preventDefault();e.stopPropagation();
    const fromIdx=parseInt(cd.dataset.idx);
    cd.classList.add('dragging');
    const onMove=ev=>{
      ev.preventDefault();
      // 모든 grid 내 카드에서 hover 대상 찾기
      document.querySelectorAll('.cd.dragover').forEach(c=>c.classList.remove('dragover'));
      const target=document.elementFromPoint(ev.clientX,ev.clientY);
      const targetCard=target?target.closest('.cd[data-idx]:not(.dragging)'):null;
      if(targetCard)targetCard.classList.add('dragover');
    };
    const onUp=async ev=>{
      document.removeEventListener('pointermove',onMove);
      document.removeEventListener('pointerup',onUp);
      cd.classList.remove('dragging');
      document.querySelectorAll('.cd.dragover').forEach(c=>c.classList.remove('dragover'));
      const target=document.elementFromPoint(ev.clientX,ev.clientY);
      const targetCard=target?target.closest('.cd[data-idx]:not(.dragging)'):null;
      if(targetCard){
        const toIdx=parseInt(targetCard.dataset.idx);
        if(fromIdx!==toIdx)await invoke('move_memo_position',{from:fromIdx,to:toIdx});
      }
    };
    document.addEventListener('pointermove',onMove);
    document.addEventListener('pointerup',onUp);
  });
}

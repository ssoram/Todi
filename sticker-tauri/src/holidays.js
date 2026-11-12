// ═══ 대한민국 공휴일 ═══
// 양력 고정 공휴일은 매년 계산하고, 음력 공휴일(설날·추석·부처님오신날)은 아래 표를 쓴다.
// 표에 없는 연도는 양력 공휴일만 표시된다. (연도를 늘릴 때는 H_LUNAR만 추가)

// 양력 고정 공휴일 (MM-DD)
const H_FIXED={
  '01-01':'신정','03-01':'삼일절','05-05':'어린이날','06-06':'현충일',
  '08-15':'광복절','10-03':'개천절','10-09':'한글날','12-25':'크리스마스',
};

// 음력 공휴일의 양력 날짜 — seol: 설날, chuseok: 추석, buddha: 부처님오신날
const H_LUNAR={
  2024:{seol:'2024-02-10',chuseok:'2024-09-17',buddha:'2024-05-15'},
  2025:{seol:'2025-01-29',chuseok:'2025-10-06',buddha:'2025-05-05'},
  2026:{seol:'2026-02-17',chuseok:'2026-09-25',buddha:'2026-05-24'},
  2027:{seol:'2027-02-07',chuseok:'2027-09-15',buddha:'2027-05-13'},
  2028:{seol:'2028-01-26',chuseok:'2028-10-03',buddha:'2028-05-02'},
  2029:{seol:'2029-02-13',chuseok:'2029-09-22',buddha:'2029-05-20'},
  2030:{seol:'2030-02-03',chuseok:'2030-09-12',buddha:'2030-05-09'},
  2031:{seol:'2031-01-23',chuseok:'2031-10-01',buddha:'2031-05-28'},
  2032:{seol:'2032-02-11',chuseok:'2032-09-19',buddha:'2032-05-16'},
  2033:{seol:'2033-01-31',chuseok:'2033-09-08',buddha:'2033-05-06'},
  2034:{seol:'2034-02-19',chuseok:'2034-09-27',buddha:'2034-05-25'},
  2035:{seol:'2035-02-08',chuseok:'2035-09-16',buddha:'2035-05-15'},
};

// 임시공휴일 · 선거일 (새로 지정될 때마다 직접 추가)
const H_EXTRA={
  '2024-04-10':'국회의원선거',
  '2024-10-01':'국군의날',
  '2025-01-27':'임시공휴일',
  '2025-06-03':'대통령선거',
  '2026-06-03':'지방선거',
};

// 대체공휴일 대상 (신정·현충일·임시공휴일은 대상이 아니다)
const H_SUB=['삼일절','어린이날','부처님오신날','광복절','개천절','한글날','크리스마스'];

function _hd(s){const p=s.split('-');return new Date(+p[0],+p[1]-1,+p[2])}
function _hs(dt){return `${dt.getFullYear()}-${String(dt.getMonth()+1).padStart(2,'0')}-${String(dt.getDate()).padStart(2,'0')}`}
function _hAdd(s,n){const dt=_hd(s);dt.setDate(dt.getDate()+n);return _hs(dt)}
function _hDow(s){return _hd(s).getDay()}

const _holCache={};

// 해당 연도의 { '2026-03-01': '삼일절', … } 맵을 만든다
function buildHolidays(y){
  // 1) 공휴일 목록 수집 (같은 날에 둘이 겹칠 수 있으므로 배열로)
  const list=[];
  Object.keys(H_FIXED).forEach(md=>list.push({ds:`${y}-${md}`,nm:H_FIXED[md]}));
  const L=H_LUNAR[y];
  const runs=[]; // 설날·추석 연휴 (3일)
  if(L){
    [[L.seol,'설날','설 연휴'],[L.chuseok,'추석','추석 연휴']].forEach(([mid,nm,side])=>{
      const days=[_hAdd(mid,-1),mid,_hAdd(mid,1)];
      list.push({ds:days[0],nm:side},{ds:days[1],nm},{ds:days[2],nm:side});
      runs.push(days);
    });
    list.push({ds:L.buddha,nm:'부처님오신날'});
  }
  Object.keys(H_EXTRA).forEach(ds=>{if(ds.startsWith(y+'-'))list.push({ds,nm:H_EXTRA[ds]})});

  // 2) 날짜별로 합치기 (겹치면 이름을 함께 표시)
  const map={};
  list.sort((a,b)=>a.ds<b.ds?-1:a.ds>b.ds?1:0);
  list.forEach(h=>{map[h.ds]=map[h.ds]?map[h.ds]+'·'+h.nm:h.nm});

  // 3) 대체공휴일 — 일요일은 그 자체로 공휴일이라 건너뛰고 공휴일이 아닌 첫 날을 찾는다
  const taken={...map};
  const nextOff=ds=>{let d=_hAdd(ds,1);while(_hDow(d)===0||taken[d])d=_hAdd(d,1);return d};
  const subs=[];
  const runDays=new Set(runs.flat());

  // 설날·추석 연휴: 일요일 또는 다른 공휴일과 겹치면 연휴 다음 첫 비공휴일 하루
  runs.forEach(days=>{
    const clash=days.some(d=>_hDow(d)===0)||days.some(d=>map[d].includes('·'));
    if(clash)subs.push({anchor:days[2]});
  });
  // 그 외 대상: 토·일요일이나 다른 공휴일과 겹치면 다음 첫 비공휴일
  const seen={};
  list.forEach(h=>{
    if(!H_SUB.includes(h.nm)||runDays.has(h.ds))return; // 연휴와 겹친 건은 위에서 처리됨
    const dow=_hDow(h.ds);
    const overlap=!!seen[h.ds]; // 같은 날에 이미 다른 공휴일이 있음
    seen[h.ds]=1;
    if(dow===0||dow===6||overlap)subs.push({anchor:h.ds});
  });
  // 앞선 날짜부터 차례로 배정해 같은 날에 몰리지 않게 한다
  subs.sort((a,b)=>a.anchor<b.anchor?-1:1).forEach(s=>{
    const d=nextOff(s.anchor);
    taken[d]='대체공휴일';map[d]='대체공휴일';
  });

  return map;
}

// 날짜 문자열('YYYY-MM-DD')의 공휴일 이름 — 공휴일이 아니면 null
function holidayName(ds){
  if(!ds)return null;
  const y=+ds.slice(0,4);
  if(!_holCache[y])_holCache[y]=buildHolidays(y);
  return _holCache[y][ds]||null;
}

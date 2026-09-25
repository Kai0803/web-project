// 以事件委派取代 inline handler，讓 CSP 能維持 script-src 'self'
document.addEventListener('submit', (event) => {
  const message = event.target.dataset.confirm;
  if (message && !window.confirm(message)) {
    event.preventDefault();
  }
});

document.addEventListener('change', (event) => {
  if (event.target.dataset.autosubmit !== undefined) {
    event.target.form.submit();
  }
});

// 完工簽名板：支援觸控與滑鼠，送出前轉成 PNG data URL
(function initSignaturePad() {
  const canvas = document.getElementById('signature-pad');
  const form = document.getElementById('signature-form');
  if (!canvas || !form) return;

  const ctx = canvas.getContext('2d');
  let drawing = false;
  let hasStrokes = false;

  // 依實際顯示寬度調整解析度，避免在手機上簽名歪掉
  function resize() {
    const ratio = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * ratio;
    canvas.height = rect.height * ratio;
    ctx.scale(ratio, ratio);
    ctx.lineWidth = 2;
    ctx.lineCap = 'round';
    ctx.strokeStyle = '#1f2933';
  }
  resize();

  function positionOf(event) {
    const rect = canvas.getBoundingClientRect();
    const point = event.touches ? event.touches[0] : event;
    return { x: point.clientX - rect.left, y: point.clientY - rect.top };
  }

  function start(event) {
    event.preventDefault();
    drawing = true;
    hasStrokes = true;
    const { x, y } = positionOf(event);
    ctx.beginPath();
    ctx.moveTo(x, y);
  }

  function move(event) {
    if (!drawing) return;
    event.preventDefault();
    const { x, y } = positionOf(event);
    ctx.lineTo(x, y);
    ctx.stroke();
  }

  function end() {
    drawing = false;
  }

  canvas.addEventListener('mousedown', start);
  canvas.addEventListener('mousemove', move);
  window.addEventListener('mouseup', end);
  canvas.addEventListener('touchstart', start, { passive: false });
  canvas.addEventListener('touchmove', move, { passive: false });
  canvas.addEventListener('touchend', end);

  document.getElementById('signature-clear').addEventListener('click', () => {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    hasStrokes = false;
  });

  form.addEventListener('submit', (event) => {
    if (!hasStrokes) {
      event.preventDefault();
      window.alert('請先請客戶簽名');
      return;
    }
    if (!window.confirm('送出後工單將標記為已完成，確定嗎？')) {
      event.preventDefault();
      return;
    }
    document.getElementById('signature-data').value = canvas.toDataURL('image/png');
  });
})();

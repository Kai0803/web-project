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

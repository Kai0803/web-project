const express = require('express');
const router = express.Router();
const bcrypt = require('bcryptjs');
const rateLimit = require('express-rate-limit');
const db = require('../db');
const { requireAuth } = require('../middleware/auth');

const MAX_FAILED_ATTEMPTS = 5;
const LOCK_MINUTES = 15;

// 同一來源 IP 的登入嘗試上限，擋自動化猜密碼
const loginLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  max: 20,
  standardHeaders: true,
  legacyHeaders: false,
  handler: (req, res) =>
    res.status(429).render('auth/login', {
      title: '員工登入',
      error: '登入嘗試過於頻繁，請稍後再試。',
      username: req.body.username || '',
    }),
});

function renderLoginError(res, status, error, username) {
  return res.status(status).render('auth/login', { title: '員工登入', error, username: username || '' });
}

router.get('/login', (req, res) => {
  if (req.session.user) return res.redirect('/admin/dashboard');
  res.render('auth/login', { title: '員工登入', error: null, username: '' });
});

router.post('/login', loginLimiter, (req, res) => {
  const { username, password } = req.body;
  const user = db
    .prepare('SELECT * FROM users WHERE username = ? AND active = 1')
    .get((username || '').trim());

  // 帳號鎖定中
  if (user && user.locked_until && new Date(user.locked_until) > new Date()) {
    return renderLoginError(res, 429, `密碼錯誤次數過多，帳號已暫時鎖定，請於 ${LOCK_MINUTES} 分鐘後再試。`, username);
  }

  if (!user || !bcrypt.compareSync(password || '', user.password_hash)) {
    if (user) {
      const attempts = user.failed_attempts + 1;
      if (attempts >= MAX_FAILED_ATTEMPTS) {
        const until = new Date(Date.now() + LOCK_MINUTES * 60 * 1000).toISOString();
        db.prepare('UPDATE users SET failed_attempts = 0, locked_until = ? WHERE id = ?').run(until, user.id);
        return renderLoginError(res, 429, `密碼錯誤次數過多，帳號已鎖定 ${LOCK_MINUTES} 分鐘。`, username);
      }
      db.prepare('UPDATE users SET failed_attempts = ? WHERE id = ?').run(attempts, user.id);
    }
    // 不論帳號是否存在都回相同訊息，避免洩漏哪些帳號存在
    return renderLoginError(res, 401, '帳號或密碼錯誤', username);
  }

  db.prepare('UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = ?').run(user.id);

  const returnTo = req.session.returnTo;
  // 重新產生 session id，避免 session fixation
  req.session.regenerate((err) => {
    if (err) return renderLoginError(res, 500, '登入失敗，請再試一次。', username);

    req.session.user = {
      id: user.id,
      name: user.name,
      username: user.username,
      role: user.role,
      mustChangePassword: !!user.must_change_password,
    };

    if (user.must_change_password) return res.redirect('/change-password');
    res.redirect(returnTo || '/admin/dashboard');
  });
});

router.get('/change-password', requireAuth, (req, res) => {
  res.render('auth/change-password', {
    title: '變更密碼',
    error: null,
    forced: !!req.session.user.mustChangePassword,
  });
});

router.post('/change-password', requireAuth, (req, res) => {
  const { current_password, new_password, confirm_password } = req.body;
  const forced = !!req.session.user.mustChangePassword;
  const fail = (error) => res.status(400).render('auth/change-password', { title: '變更密碼', error, forced });

  const user = db.prepare('SELECT * FROM users WHERE id = ?').get(req.session.user.id);
  if (!user) return res.redirect('/logout');

  if (!bcrypt.compareSync(current_password || '', user.password_hash)) return fail('目前密碼不正確');
  if (!new_password || new_password.length < 8) return fail('新密碼至少需要 8 個字元');
  if (new_password !== confirm_password) return fail('兩次輸入的新密碼不一致');
  if (bcrypt.compareSync(new_password, user.password_hash)) return fail('新密碼不可與目前密碼相同');

  db.prepare('UPDATE users SET password_hash = ?, must_change_password = 0 WHERE id = ?').run(
    bcrypt.hashSync(new_password, 10),
    user.id
  );
  req.session.user.mustChangePassword = false;
  res.redirect('/admin/dashboard');
});

router.post('/logout', (req, res) => {
  req.session.destroy(() => {
    res.clearCookie('ac.sid');
    res.redirect('/login');
  });
});

module.exports = router;

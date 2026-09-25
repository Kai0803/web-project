const express = require('express');
const router = express.Router();
const bcrypt = require('bcryptjs');
const db = require('../db');
const { requireAuth, requireRole } = require('../middleware/auth');

router.use(requireAuth, requireRole('admin'));

router.get('/users', (req, res) => {
  const users = db.prepare('SELECT * FROM users ORDER BY role, name').all();
  res.render('admin/users', {
    title: '人員管理',
    active: 'users',
    user: req.session.user,
    users,
    error: null,
  });
});

router.post('/users', (req, res) => {
  const { name, username, password, role, phone } = req.body;
  if (!name || !username || !password || !['admin', 'engineer'].includes(role)) {
    const users = db.prepare('SELECT * FROM users ORDER BY role, name').all();
    return res.status(400).render('admin/users', {
      title: '人員管理',
      active: 'users',
      user: req.session.user,
      users,
      error: '請填寫完整資料',
    });
  }

  try {
    // 由管理員設定的初始密碼，要求本人首次登入時自行更換
    db.prepare(`
      INSERT INTO users (name, username, password_hash, role, phone, must_change_password)
      VALUES (?, ?, ?, ?, ?, 1)
    `).run(name.trim(), username.trim(), bcrypt.hashSync(password, 10), role, phone ? phone.trim() : null);
    res.redirect('/admin/users');
  } catch (e) {
    const users = db.prepare('SELECT * FROM users ORDER BY role, name').all();
    res.status(400).render('admin/users', {
      title: '人員管理',
      active: 'users',
      user: req.session.user,
      users,
      error: '帳號已存在，請使用其他帳號名稱',
    });
  }
});

router.post('/users/:id/toggle-active', (req, res) => {
  const target = db.prepare('SELECT * FROM users WHERE id = ?').get(req.params.id);
  if (target) {
    db.prepare('UPDATE users SET active = ? WHERE id = ?').run(target.active ? 0 : 1, target.id);
  }
  res.redirect('/admin/users');
});

router.post('/users/:id/reset-password', (req, res) => {
  const { password } = req.body;
  if (password && password.length >= 8) {
    // 一併解除鎖定，並要求對方下次登入自行更換
    db.prepare(
      'UPDATE users SET password_hash = ?, must_change_password = 1, failed_attempts = 0, locked_until = NULL WHERE id = ?'
    ).run(bcrypt.hashSync(password, 10), req.params.id);
  }
  res.redirect('/admin/users');
});

module.exports = router;

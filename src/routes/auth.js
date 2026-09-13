const express = require('express');
const router = express.Router();
const bcrypt = require('bcryptjs');
const db = require('../db');

router.get('/login', (req, res) => {
  if (req.session.user) return res.redirect('/admin/dashboard');
  res.render('auth/login', { title: '員工登入', error: null, username: '' });
});

router.post('/login', (req, res) => {
  const { username, password } = req.body;
  const user = db
    .prepare('SELECT * FROM users WHERE username = ? AND active = 1')
    .get((username || '').trim());

  if (!user || !bcrypt.compareSync(password || '', user.password_hash)) {
    return res.status(401).render('auth/login', {
      title: '員工登入',
      error: '帳號或密碼錯誤',
      username: username || '',
    });
  }

  req.session.user = {
    id: user.id,
    name: user.name,
    username: user.username,
    role: user.role,
  };

  const returnTo = req.session.returnTo;
  delete req.session.returnTo;
  res.redirect(returnTo || '/admin/dashboard');
});

router.get('/logout', (req, res) => {
  req.session.destroy(() => {
    res.redirect('/login');
  });
});

module.exports = router;

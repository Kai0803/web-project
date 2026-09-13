const express = require('express');
const router = express.Router();
const db = require('../db');
const { requireAuth, requireRole } = require('../middleware/auth');
const { TOOL_STATUS_LABELS } = require('../utils/helpers');

router.use(requireAuth);

router.get('/tools', (req, res) => {
  const tools = db
    .prepare(`
      SELECT t.*, u.name AS holder_name
      FROM tools t
      LEFT JOIN users u ON u.id = t.current_holder_id
      ORDER BY t.category, t.code
    `)
    .all();
  res.render('admin/tools', {
    title: '工具管理',
    active: 'tools',
    user: req.session.user,
    tools,
    TOOL_STATUS_LABELS,
    editTool: null,
  });
});

router.get('/tools/:id/edit', requireRole('admin'), (req, res) => {
  const tools = db
    .prepare(`
      SELECT t.*, u.name AS holder_name
      FROM tools t
      LEFT JOIN users u ON u.id = t.current_holder_id
      ORDER BY t.category, t.code
    `)
    .all();
  const editTool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  res.render('admin/tools', {
    title: '工具管理',
    active: 'tools',
    user: req.session.user,
    tools,
    TOOL_STATUS_LABELS,
    editTool,
  });
});

router.post('/tools', requireRole('admin'), (req, res) => {
  const { name, code, category, note } = req.body;
  if (!name || !name.trim() || !code || !code.trim()) return res.redirect('/admin/tools');
  db.prepare('INSERT INTO tools (name, code, category, note) VALUES (?, ?, ?, ?)').run(
    name.trim(),
    code.trim(),
    category ? category.trim() : null,
    note ? note.trim() : null
  );
  res.redirect('/admin/tools');
});

router.post('/tools/:id', requireRole('admin'), (req, res) => {
  const { name, code, category, note } = req.body;
  db.prepare('UPDATE tools SET name = ?, code = ?, category = ?, note = ? WHERE id = ?').run(
    name.trim(),
    code.trim(),
    category ? category.trim() : null,
    note ? note.trim() : null,
    req.params.id
  );
  res.redirect('/admin/tools');
});

router.post('/tools/:id/checkout', (req, res) => {
  const user = req.session.user;
  const tool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  if (!tool || tool.status !== 'available') return res.redirect('/admin/tools');

  const tx = db.transaction(() => {
    db.prepare("UPDATE tools SET status = 'in_use', current_holder_id = ? WHERE id = ?").run(user.id, tool.id);
    db.prepare('INSERT INTO tool_logs (tool_id, engineer_id, action) VALUES (?, ?, ?)').run(tool.id, user.id, 'checkout');
  });
  tx();
  res.redirect('/admin/tools');
});

router.post('/tools/:id/return', (req, res) => {
  const user = req.session.user;
  const tool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  if (!tool) return res.redirect('/admin/tools');
  if (user.role !== 'admin' && tool.current_holder_id !== user.id) {
    return res.status(403).render('error', { title: '權限不足', message: '您沒有權限歸還此工具', user });
  }

  const tx = db.transaction(() => {
    db.prepare("UPDATE tools SET status = 'available', current_holder_id = NULL WHERE id = ?").run(tool.id);
    db.prepare('INSERT INTO tool_logs (tool_id, engineer_id, action) VALUES (?, ?, ?)').run(tool.id, user.id, 'return');
  });
  tx();
  res.redirect('/admin/tools');
});

router.post('/tools/:id/maintenance', requireRole('admin'), (req, res) => {
  const user = req.session.user;
  const tool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  if (!tool) return res.redirect('/admin/tools');

  const tx = db.transaction(() => {
    db.prepare("UPDATE tools SET status = 'maintenance', current_holder_id = NULL WHERE id = ?").run(tool.id);
    db.prepare('INSERT INTO tool_logs (tool_id, engineer_id, action) VALUES (?, ?, ?)').run(tool.id, user.id, 'maintenance');
  });
  tx();
  res.redirect('/admin/tools');
});

router.post('/tools/:id/available', requireRole('admin'), (req, res) => {
  const user = req.session.user;
  const tool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  if (!tool) return res.redirect('/admin/tools');

  const tx = db.transaction(() => {
    db.prepare("UPDATE tools SET status = 'available', current_holder_id = NULL WHERE id = ?").run(tool.id);
    db.prepare('INSERT INTO tool_logs (tool_id, engineer_id, action) VALUES (?, ?, ?)').run(tool.id, user.id, 'available');
  });
  tx();
  res.redirect('/admin/tools');
});

router.post('/tools/:id/retire', requireRole('admin'), (req, res) => {
  const user = req.session.user;
  const tool = db.prepare('SELECT * FROM tools WHERE id = ?').get(req.params.id);
  if (!tool) return res.redirect('/admin/tools');

  const tx = db.transaction(() => {
    db.prepare("UPDATE tools SET status = 'retired', current_holder_id = NULL WHERE id = ?").run(tool.id);
    db.prepare('INSERT INTO tool_logs (tool_id, engineer_id, action) VALUES (?, ?, ?)').run(tool.id, user.id, 'retired');
  });
  tx();
  res.redirect('/admin/tools');
});

module.exports = router;

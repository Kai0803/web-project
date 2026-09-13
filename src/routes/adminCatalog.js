const express = require('express');
const router = express.Router();
const db = require('../db');
const { requireAuth, requireRole } = require('../middleware/auth');
const { formatMoney } = require('../utils/helpers');

router.use(requireAuth);

// ================= Service Items (工項) =================
router.get('/service-items', requireRole('admin'), (req, res) => {
  const items = db.prepare('SELECT * FROM service_items ORDER BY category, id').all();
  res.render('admin/service-items', {
    title: '服務項目（工項）',
    active: 'service-items',
    user: req.session.user,
    items,
    formatMoney,
    editItem: null,
  });
});

router.get('/service-items/:id/edit', requireRole('admin'), (req, res) => {
  const items = db.prepare('SELECT * FROM service_items ORDER BY category, id').all();
  const editItem = db.prepare('SELECT * FROM service_items WHERE id = ?').get(req.params.id);
  res.render('admin/service-items', {
    title: '服務項目（工項）',
    active: 'service-items',
    user: req.session.user,
    items,
    formatMoney,
    editItem,
  });
});

router.post('/service-items', requireRole('admin'), (req, res) => {
  const { name, category, unit, base_price, est_minutes, is_public, note } = req.body;
  if (!name || !name.trim()) return res.redirect('/admin/service-items');

  db.prepare(`
    INSERT INTO service_items (name, category, unit, base_price, est_minutes, is_public, note)
    VALUES (?, ?, ?, ?, ?, ?, ?)
  `).run(
    name.trim(),
    (category || '其他').trim(),
    (unit || '次').trim(),
    parseFloat(base_price) || 0,
    parseInt(est_minutes, 10) || 60,
    is_public ? 1 : 0,
    note ? note.trim() : null
  );
  res.redirect('/admin/service-items');
});

router.post('/service-items/:id', requireRole('admin'), (req, res) => {
  const { name, category, unit, base_price, est_minutes, is_public, note } = req.body;
  db.prepare(`
    UPDATE service_items SET name = ?, category = ?, unit = ?, base_price = ?, est_minutes = ?, is_public = ?, note = ?
    WHERE id = ?
  `).run(
    name.trim(),
    (category || '其他').trim(),
    (unit || '次').trim(),
    parseFloat(base_price) || 0,
    parseInt(est_minutes, 10) || 60,
    is_public ? 1 : 0,
    note ? note.trim() : null,
    req.params.id
  );
  res.redirect('/admin/service-items');
});

router.post('/service-items/:id/delete', requireRole('admin'), (req, res) => {
  db.prepare('DELETE FROM service_items WHERE id = ?').run(req.params.id);
  res.redirect('/admin/service-items');
});

// ================= Parts (零件) =================
router.get('/parts', (req, res) => {
  const parts = db.prepare('SELECT * FROM parts ORDER BY name').all();
  res.render('admin/parts', {
    title: '零件庫存',
    active: 'parts',
    user: req.session.user,
    parts,
    formatMoney,
    editPart: null,
  });
});

router.get('/parts/:id/edit', requireRole('admin'), (req, res) => {
  const parts = db.prepare('SELECT * FROM parts ORDER BY name').all();
  const editPart = db.prepare('SELECT * FROM parts WHERE id = ?').get(req.params.id);
  res.render('admin/parts', {
    title: '零件庫存',
    active: 'parts',
    user: req.session.user,
    parts,
    formatMoney,
    editPart,
  });
});

router.post('/parts', requireRole('admin'), (req, res) => {
  const { name, spec, unit, stock_qty, unit_price, low_stock_threshold, note } = req.body;
  if (!name || !name.trim()) return res.redirect('/admin/parts');

  db.prepare(`
    INSERT INTO parts (name, spec, unit, stock_qty, unit_price, low_stock_threshold, note)
    VALUES (?, ?, ?, ?, ?, ?, ?)
  `).run(
    name.trim(),
    spec ? spec.trim() : null,
    (unit || '個').trim(),
    parseInt(stock_qty, 10) || 0,
    parseFloat(unit_price) || 0,
    parseInt(low_stock_threshold, 10) || 5,
    note ? note.trim() : null
  );
  res.redirect('/admin/parts');
});

router.post('/parts/:id', requireRole('admin'), (req, res) => {
  const { name, spec, unit, stock_qty, unit_price, low_stock_threshold, note } = req.body;
  db.prepare(`
    UPDATE parts SET name = ?, spec = ?, unit = ?, stock_qty = ?, unit_price = ?, low_stock_threshold = ?, note = ?
    WHERE id = ?
  `).run(
    name.trim(),
    spec ? spec.trim() : null,
    (unit || '個').trim(),
    parseInt(stock_qty, 10) || 0,
    parseFloat(unit_price) || 0,
    parseInt(low_stock_threshold, 10) || 5,
    note ? note.trim() : null,
    req.params.id
  );
  res.redirect('/admin/parts');
});

router.post('/parts/:id/delete', requireRole('admin'), (req, res) => {
  db.prepare('DELETE FROM parts WHERE id = ?').run(req.params.id);
  res.redirect('/admin/parts');
});

module.exports = router;

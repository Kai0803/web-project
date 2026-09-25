const express = require('express');
const router = express.Router();
const db = require('../db');
const { requireAuth, requireRole } = require('../middleware/auth');
const { saveSignature, photoPath, removePhoto } = require('../middleware/upload');
const { STATUS_LABELS, STATUS_BADGE_CLASS, TOOL_STATUS_LABELS, formatMoney } = require('../utils/helpers');

router.use(requireAuth);

function recomputeTotal(orderId) {
  const itemsTotal = db
    .prepare('SELECT COALESCE(SUM(qty * unit_price), 0) AS t FROM order_items WHERE order_id = ?')
    .get(orderId).t;
  const partsTotal = db
    .prepare('SELECT COALESCE(SUM(qty * unit_price), 0) AS t FROM order_parts WHERE order_id = ?')
    .get(orderId).t;
  db.prepare("UPDATE orders SET total_amount = ?, updated_at = datetime('now') WHERE id = ?").run(
    itemsTotal + partsTotal,
    orderId
  );
}

function canAccessOrder(user, order) {
  if (user.role === 'admin') return true;
  return order.assigned_engineer_id === user.id;
}

// ---- Dashboard ----
router.get('/dashboard', (req, res) => {
  const user = req.session.user;
  if (user.role === 'admin') {
    const pendingCount = db.prepare("SELECT COUNT(*) AS c FROM orders WHERE status = 'pending'").get().c;
    const todayCount = db
      .prepare("SELECT COUNT(*) AS c FROM orders WHERE scheduled_date = date('now') AND status IN ('scheduled','in_progress')")
      .get().c;
    const lowStock = db
      .prepare('SELECT * FROM parts WHERE stock_qty <= low_stock_threshold ORDER BY stock_qty ASC')
      .all();
    const maintenanceTools = db.prepare("SELECT COUNT(*) AS c FROM tools WHERE status = 'maintenance'").get().c;
    const recentOrders = db
      .prepare("SELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC LIMIT 8")
      .all();
    return res.render('admin/dashboard', {
      title: '儀表板',
      active: 'dashboard',
      user,
      isAdmin: true,
      pendingCount,
      todayCount,
      lowStock,
      maintenanceTools,
      recentOrders,
      STATUS_LABELS,
      STATUS_BADGE_CLASS,
      formatMoney,
    });
  }

  const myToday = db
    .prepare(
      "SELECT * FROM orders WHERE assigned_engineer_id = ? AND scheduled_date = date('now') ORDER BY scheduled_time"
    )
    .all(user.id);
  const myUpcoming = db
    .prepare(
      "SELECT * FROM orders WHERE assigned_engineer_id = ? AND status IN ('scheduled','in_progress') AND (scheduled_date IS NULL OR scheduled_date > date('now')) ORDER BY scheduled_date"
    )
    .all(user.id);
  const myTools = db.prepare('SELECT * FROM tools WHERE current_holder_id = ?').all(user.id);

  res.render('admin/dashboard', {
    title: '儀表板',
    active: 'dashboard',
    user,
    isAdmin: false,
    myToday,
    myUpcoming,
    myTools,
    STATUS_LABELS,
    STATUS_BADGE_CLASS,
    TOOL_STATUS_LABELS,
    formatMoney,
  });
});

// ---- Orders list ----
router.get('/orders', (req, res) => {
  const user = req.session.user;
  const statusFilter = req.query.status || '';
  let orders;

  if (user.role === 'admin') {
    if (statusFilter) {
      orders = db
        .prepare('SELECT o.*, u.name AS engineer_name FROM orders o LEFT JOIN users u ON u.id = o.assigned_engineer_id WHERE o.status = ? ORDER BY o.created_at DESC')
        .all(statusFilter);
    } else {
      orders = db
        .prepare('SELECT o.*, u.name AS engineer_name FROM orders o LEFT JOIN users u ON u.id = o.assigned_engineer_id ORDER BY o.created_at DESC')
        .all();
    }
  } else {
    orders = db
      .prepare('SELECT o.*, u.name AS engineer_name FROM orders o LEFT JOIN users u ON u.id = o.assigned_engineer_id WHERE o.assigned_engineer_id = ? ORDER BY o.created_at DESC')
      .all(user.id);
  }

  res.render('admin/orders', {
    title: '預約 / 工單',
    active: 'orders',
    user,
    orders,
    statusFilter,
    STATUS_LABELS,
    STATUS_BADGE_CLASS,
    formatMoney,
  });
});

// ---- Order detail ----
/** 組裝工單頁所需的全部資料，供正常顯示與帶錯誤訊息重新顯示共用 */
function renderOrderDetail(req, res, order, extra = {}, status = 200) {
  const photos = db
    .prepare("SELECT * FROM order_photos WHERE order_id = ? AND kind != 'signature' ORDER BY created_at")
    .all(order.id);
  const signature = db
    .prepare("SELECT * FROM order_photos WHERE order_id = ? AND kind = 'signature' ORDER BY created_at DESC LIMIT 1")
    .get(order.id);

  res.status(status).render('admin/order-detail', {
    title: `工單 ${order.booking_code}`,
    active: 'orders',
    user: req.session.user,
    order,
    orderItems: db.prepare('SELECT * FROM order_items WHERE order_id = ?').all(order.id),
    orderParts: db.prepare('SELECT * FROM order_parts WHERE order_id = ?').all(order.id),
    engineers: db.prepare("SELECT id, name FROM users WHERE role = 'engineer' AND active = 1 ORDER BY name").all(),
    serviceItems: db.prepare('SELECT * FROM service_items ORDER BY category, id').all(),
    parts: db.prepare('SELECT * FROM parts ORDER BY name').all(),
    engineer: order.assigned_engineer_id
      ? db.prepare('SELECT * FROM users WHERE id = ?').get(order.assigned_engineer_id)
      : null,
    photos,
    signature,
    partError: null,
    photoError: null,
    STATUS_LABELS,
    STATUS_BADGE_CLASS,
    formatMoney,
    ...extra,
  });
}

/** 取出工單並檢查存取權限，失敗時直接回應錯誤頁並回傳 null */
function loadOrder(req, res) {
  const user = req.session.user;
  const order = db.prepare('SELECT * FROM orders WHERE id = ?').get(req.params.id);
  if (!order) {
    res.status(404).render('error', { title: '找不到工單', message: '找不到此工單', user });
    return null;
  }
  if (!canAccessOrder(user, order)) {
    res.status(403).render('error', { title: '權限不足', message: '您沒有權限操作此工單', user });
    return null;
  }
  return order;
}

router.get('/orders/:id', (req, res) => {
  const order = loadOrder(req, res);
  if (order) renderOrderDetail(req, res, order);
});

// ---- Assign engineer / schedule (admin only) ----
router.post('/orders/:id/assign', requireRole('admin'), (req, res) => {
  const { assigned_engineer_id, scheduled_date, scheduled_time } = req.body;
  const order = db.prepare('SELECT * FROM orders WHERE id = ?').get(req.params.id);
  if (!order) return res.status(404).render('error', { title: '找不到工單', message: '找不到此工單', user: req.session.user });

  const nextStatus = order.status === 'pending' ? 'scheduled' : order.status;
  db.prepare(
    `UPDATE orders SET assigned_engineer_id = ?, scheduled_date = ?, scheduled_time = ?, status = ?, updated_at = datetime('now') WHERE id = ?`
  ).run(assigned_engineer_id || null, scheduled_date || null, scheduled_time || null, nextStatus, order.id);

  res.redirect(`/admin/orders/${order.id}`);
});

// ---- Update status ----
router.post('/orders/:id/status', (req, res) => {
  const user = req.session.user;
  const order = db.prepare('SELECT * FROM orders WHERE id = ?').get(req.params.id);
  if (!order) return res.status(404).render('error', { title: '找不到工單', message: '找不到此工單', user });
  if (!canAccessOrder(user, order)) {
    return res.status(403).render('error', { title: '權限不足', message: '您沒有權限操作此工單', user });
  }

  const { status } = req.body;
  const allowed = user.role === 'admin'
    ? Object.keys(STATUS_LABELS)
    : ['in_progress', 'completed'];

  if (!allowed.includes(status)) {
    return res.status(400).render('error', { title: '操作無效', message: '無效的狀態變更', user });
  }

  db.prepare("UPDATE orders SET status = ?, updated_at = datetime('now') WHERE id = ?").run(status, order.id);
  res.redirect(`/admin/orders/${order.id}`);
});

// ---- Cancel (admin only) ----
router.post('/orders/:id/cancel', requireRole('admin'), (req, res) => {
  db.prepare("UPDATE orders SET status = 'cancelled', updated_at = datetime('now') WHERE id = ?").run(req.params.id);
  res.redirect(`/admin/orders/${req.params.id}`);
});

// ---- Add service item to order (admin only) ----
router.post('/orders/:id/items/add', requireRole('admin'), (req, res) => {
  const order = db.prepare('SELECT * FROM orders WHERE id = ?').get(req.params.id);
  if (!order) return res.status(404).render('error', { title: '找不到工單', message: '找不到此工單', user: req.session.user });

  const { service_item_id, qty } = req.body;
  const item = db.prepare('SELECT * FROM service_items WHERE id = ?').get(service_item_id);
  if (!item) return res.redirect(`/admin/orders/${order.id}`);

  const quantity = Math.max(1, parseInt(qty, 10) || 1);
  db.prepare(
    'INSERT INTO order_items (order_id, service_item_id, name_snapshot, qty, unit_price) VALUES (?, ?, ?, ?, ?)'
  ).run(order.id, item.id, item.name, quantity, item.base_price);

  recomputeTotal(order.id);
  res.redirect(`/admin/orders/${order.id}`);
});

router.post('/orders/:id/items/:itemId/remove', requireRole('admin'), (req, res) => {
  db.prepare('DELETE FROM order_items WHERE id = ? AND order_id = ?').run(req.params.itemId, req.params.id);
  recomputeTotal(req.params.id);
  res.redirect(`/admin/orders/${req.params.id}`);
});

// ---- Add / remove part usage (admin or assigned engineer) ----
router.post('/orders/:id/parts/add', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  const { part_id, qty } = req.body;
  const quantity = Math.max(1, parseInt(qty, 10) || 1);
  const part = db.prepare('SELECT * FROM parts WHERE id = ?').get(part_id);
  if (!part) return res.redirect(`/admin/orders/${order.id}`);

  if (part.stock_qty < quantity) {
    return renderOrderDetail(
      req,
      res,
      order,
      { partError: `庫存不足：「${part.name}」目前庫存 ${part.stock_qty} ${part.unit}` },
      400
    );
  }

  const tx = db.transaction(() => {
    db.prepare(
      'INSERT INTO order_parts (order_id, part_id, name_snapshot, qty, unit_price) VALUES (?, ?, ?, ?, ?)'
    ).run(order.id, part.id, part.name, quantity, part.unit_price);
    db.prepare('UPDATE parts SET stock_qty = stock_qty - ? WHERE id = ?').run(quantity, part.id);
  });
  tx();

  recomputeTotal(order.id);
  res.redirect(`/admin/orders/${order.id}`);
});

router.post('/orders/:id/parts/:partUsageId/remove', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  const usage = db.prepare('SELECT * FROM order_parts WHERE id = ? AND order_id = ?').get(req.params.partUsageId, order.id);
  if (usage) {
    const tx = db.transaction(() => {
      db.prepare('DELETE FROM order_parts WHERE id = ?').run(usage.id);
      if (usage.part_id) {
        db.prepare('UPDATE parts SET stock_qty = stock_qty + ? WHERE id = ?').run(usage.qty, usage.part_id);
      }
    });
    tx();
    recomputeTotal(order.id);
  }
  res.redirect(`/admin/orders/${order.id}`);
});

// ---- 施工照片 ----
// multer 已在 app.js 於 CSRF 檢查前先行解析此路徑的 multipart body
router.post('/orders/:id/photos', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  if (req.uploadError) {
    return renderOrderDetail(req, res, order, { photoError: req.uploadError }, 400);
  }
  const files = req.files || [];
  if (files.length === 0) {
    return renderOrderDetail(req, res, order, { photoError: '請先選擇要上傳的照片' }, 400);
  }

  const kind = ['before', 'after', 'other'].includes(req.body.kind) ? req.body.kind : 'other';
  const caption = req.body.caption ? req.body.caption.trim().slice(0, 200) : null;
  const insert = db.prepare(`
    INSERT INTO order_photos (order_id, filename, kind, caption, uploaded_by)
    VALUES (?, ?, ?, ?, ?)
  `);
  db.transaction(() => {
    for (const file of files) insert.run(order.id, file.filename, kind, caption, req.session.user.id);
  })();

  res.redirect(`/admin/orders/${order.id}`);
});

router.get('/orders/:id/photos/:photoId', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  const photo = db
    .prepare('SELECT * FROM order_photos WHERE id = ? AND order_id = ?')
    .get(req.params.photoId, order.id);
  const filePath = photo && photoPath(photo.filename);
  if (!filePath) return res.status(404).end();

  // 照片屬於客戶住家，僅限有權限存取該工單的人員檢視
  res.sendFile(filePath);
});

router.post('/orders/:id/photos/:photoId/delete', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  const photo = db
    .prepare('SELECT * FROM order_photos WHERE id = ? AND order_id = ?')
    .get(req.params.photoId, order.id);
  if (photo) {
    db.prepare('DELETE FROM order_photos WHERE id = ?').run(photo.id);
    removePhoto(photo.filename);
  }
  res.redirect(`/admin/orders/${order.id}`);
});

// ---- 客戶完工簽收 ----
router.post('/orders/:id/signature', (req, res) => {
  const order = loadOrder(req, res);
  if (!order) return;

  const filename = saveSignature(req.body.signature);
  if (!filename) {
    return renderOrderDetail(req, res, order, { photoError: '簽名內容無效，請重新簽一次' }, 400);
  }

  db.transaction(() => {
    db.prepare(`
      INSERT INTO order_photos (order_id, filename, kind, uploaded_by)
      VALUES (?, ?, 'signature', ?)
    `).run(order.id, filename, req.session.user.id);
    // 簽收即代表完工
    db.prepare(
      "UPDATE orders SET status = 'completed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?"
    ).run(order.id);
  })();

  res.redirect(`/admin/orders/${order.id}`);
});

module.exports = router;

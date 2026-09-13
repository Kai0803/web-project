const express = require('express');
const router = express.Router();
const db = require('../db');
const { generateBookingCode, STATUS_LABELS, STATUS_BADGE_CLASS, formatMoney } = require('../utils/helpers');

const TIME_SLOTS = ['上午 09:00-12:00', '下午 13:00-17:00', '晚間 17:00-20:00'];

router.get('/', (req, res) => {
  const items = db
    .prepare('SELECT * FROM service_items WHERE is_public = 1 ORDER BY category, id')
    .all();
  const grouped = {};
  for (const item of items) {
    if (!grouped[item.category]) grouped[item.category] = [];
    grouped[item.category].push(item);
  }
  res.render('public/home', { title: '首頁', grouped, formatMoney });
});

router.get('/booking', (req, res) => {
  const items = db
    .prepare('SELECT * FROM service_items WHERE is_public = 1 ORDER BY category, id')
    .all();
  const grouped = {};
  for (const item of items) {
    if (!grouped[item.category]) grouped[item.category] = [];
    grouped[item.category].push(item);
  }
  res.render('public/booking', {
    title: '線上預約',
    grouped,
    timeSlots: TIME_SLOTS,
    formatMoney,
    errors: null,
    formData: {},
  });
});

router.post('/booking', (req, res) => {
  const { customer_name, phone, address, preferred_date, preferred_time, note } = req.body;
  let serviceItemIds = req.body.service_item_ids || [];
  if (!Array.isArray(serviceItemIds)) serviceItemIds = [serviceItemIds];
  serviceItemIds = serviceItemIds.filter(Boolean).map(Number);

  const errors = [];
  if (!customer_name || !customer_name.trim()) errors.push('請填寫姓名');
  if (!phone || !/^[0-9+\-\s]{8,15}$/.test(phone.trim())) errors.push('請填寫正確的聯絡電話');
  if (!address || !address.trim()) errors.push('請填寫服務地址');
  if (!preferred_date) errors.push('請選擇希望服務日期');
  if (!preferred_time || !TIME_SLOTS.includes(preferred_time)) errors.push('請選擇希望服務時段');
  if (serviceItemIds.length === 0) errors.push('請至少選擇一項服務項目');

  const items = db
    .prepare('SELECT * FROM service_items WHERE is_public = 1 ORDER BY category, id')
    .all();

  if (errors.length > 0) {
    const grouped = {};
    for (const item of items) {
      if (!grouped[item.category]) grouped[item.category] = [];
      grouped[item.category].push(item);
    }
    return res.status(400).render('public/booking', {
      title: '線上預約',
      grouped,
      timeSlots: TIME_SLOTS,
      formatMoney,
      errors,
      formData: { customer_name, phone, address, preferred_date, preferred_time, note, serviceItemIds },
    });
  }

  const selectedItems = items.filter((i) => serviceItemIds.includes(i.id));
  const total = selectedItems.reduce((sum, i) => sum + i.base_price, 0);
  const bookingCode = generateBookingCode(db);

  const insertOrder = db.prepare(`
    INSERT INTO orders (booking_code, customer_name, phone, address, preferred_date, preferred_time, note, total_amount)
    VALUES (@booking_code, @customer_name, @phone, @address, @preferred_date, @preferred_time, @note, @total_amount)
  `);
  const insertItem = db.prepare(`
    INSERT INTO order_items (order_id, service_item_id, name_snapshot, qty, unit_price)
    VALUES (@order_id, @service_item_id, @name_snapshot, 1, @unit_price)
  `);

  const tx = db.transaction(() => {
    const info = insertOrder.run({
      booking_code: bookingCode,
      customer_name: customer_name.trim(),
      phone: phone.trim(),
      address: address.trim(),
      preferred_date,
      preferred_time,
      note: note ? note.trim() : null,
      total_amount: total,
    });
    for (const item of selectedItems) {
      insertItem.run({
        order_id: info.lastInsertRowid,
        service_item_id: item.id,
        name_snapshot: item.name,
        unit_price: item.base_price,
      });
    }
    return info.lastInsertRowid;
  });
  tx();

  res.render('public/booking-success', { title: '預約成功', bookingCode, phone: phone.trim() });
});

router.get('/booking/lookup', (req, res) => {
  res.render('public/lookup', { title: '查詢預約進度', order: null, notFound: false, query: {} });
});

router.post('/booking/lookup', (req, res) => {
  const { booking_code, phone } = req.body;
  const order = db
    .prepare('SELECT * FROM orders WHERE booking_code = ? AND phone = ?')
    .get((booking_code || '').trim().toUpperCase(), (phone || '').trim());

  if (!order) {
    return res.render('public/lookup', {
      title: '查詢預約進度',
      order: null,
      notFound: true,
      query: { booking_code, phone },
    });
  }

  const orderItems = db.prepare('SELECT * FROM order_items WHERE order_id = ?').all(order.id);
  let engineerName = null;
  if (order.assigned_engineer_id) {
    const eng = db.prepare('SELECT name, phone FROM users WHERE id = ?').get(order.assigned_engineer_id);
    engineerName = eng ? eng.name : null;
  }

  res.render('public/lookup', {
    title: '查詢預約進度',
    order,
    orderItems,
    engineerName,
    STATUS_LABELS,
    STATUS_BADGE_CLASS,
    formatMoney,
    notFound: false,
    query: { booking_code, phone },
  });
});

module.exports = router;

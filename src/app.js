require('../src/utils/seed');

const path = require('path');
const express = require('express');
const session = require('express-session');

const db = require('./db');

const publicRoutes = require('./routes/public');
const authRoutes = require('./routes/auth');
const adminOrdersRoutes = require('./routes/adminOrders');
const adminCatalogRoutes = require('./routes/adminCatalog');
const adminToolsRoutes = require('./routes/adminTools');
const adminUsersRoutes = require('./routes/adminUsers');

const app = express();

app.set('view engine', 'ejs');
app.set('views', path.join(__dirname, '..', 'views'));

app.use(express.urlencoded({ extended: true }));
app.use(express.static(path.join(__dirname, '..', 'public')));

app.use(
  session({
    secret: process.env.SESSION_SECRET || 'ac-service-dev-secret-change-me',
    resave: false,
    saveUninitialized: false,
    cookie: { maxAge: 1000 * 60 * 60 * 8 },
  })
);

app.use((req, res, next) => {
  res.locals.user = req.session.user || null;
  next();
});

// 供容器 liveness / readiness probe 使用
app.get('/healthz', (req, res) => {
  try {
    db.prepare('SELECT 1').get();
    res.json({ status: 'ok' });
  } catch (err) {
    res.status(503).json({ status: 'error' });
  }
});

app.use('/', publicRoutes);
app.use('/', authRoutes);
app.use('/admin', adminOrdersRoutes);
app.use('/admin', adminCatalogRoutes);
app.use('/admin', adminToolsRoutes);
app.use('/admin', adminUsersRoutes);

app.use((req, res) => {
  res.status(404).render('error', { title: '找不到頁面', message: '找不到您要的頁面。', user: res.locals.user });
});

const PORT = process.env.PORT || 3000;
app.listen(PORT, () => {
  console.log(`冷氣工程服務系統已啟動：http://localhost:${PORT}`);
});

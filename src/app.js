require('../src/utils/seed');

const path = require('path');
const express = require('express');
const session = require('express-session');
const helmet = require('helmet');

const db = require('./db');
const SqliteStore = require('./middleware/sessionStore');
const csrf = require('./middleware/csrf');
const { requirePasswordChanged } = require('./middleware/auth');

const publicRoutes = require('./routes/public');
const authRoutes = require('./routes/auth');
const adminOrdersRoutes = require('./routes/adminOrders');
const adminCatalogRoutes = require('./routes/adminCatalog');
const adminToolsRoutes = require('./routes/adminTools');
const adminUsersRoutes = require('./routes/adminUsers');

const app = express();

app.set('view engine', 'ejs');
app.set('views', path.join(__dirname, '..', 'views'));

// 反向代理後方（例如 nginx / ingress）才能正確判斷 secure cookie 與來源 IP
if (process.env.TRUST_PROXY === 'true') app.set('trust proxy', 1);

app.use(
  helmet({
    contentSecurityPolicy: {
      directives: {
        defaultSrc: ["'self'"],
        scriptSrc: ["'self'"],
        styleSrc: ["'self'"],
        imgSrc: ["'self'", 'data:'],
        objectSrc: ["'none'"],
        formAction: ["'self'"],
        frameAncestors: ["'none'"],
      },
    },
  })
);

app.use(express.urlencoded({ extended: true }));
app.use(express.static(path.join(__dirname, '..', 'public')));

// 供容器 liveness / readiness probe 使用。
// 放在 session middleware 之前，避免每次探測都建立一筆 session。
app.get('/healthz', (req, res) => {
  try {
    db.prepare('SELECT 1').get();
    res.json({ status: 'ok' });
  } catch (err) {
    res.status(503).json({ status: 'error' });
  }
});

app.use(
  session({
    name: 'ac.sid',
    store: new SqliteStore(),
    secret: process.env.SESSION_SECRET || 'ac-service-dev-secret-change-me',
    resave: false,
    saveUninitialized: false,
    cookie: {
      maxAge: 1000 * 60 * 60 * 8,
      httpOnly: true,
      sameSite: 'lax',
      // 透過 HTTPS 提供服務時請設定 COOKIE_SECURE=true
      secure: process.env.COOKIE_SECURE === 'true',
    },
  })
);

app.use(csrf);

app.use((req, res, next) => {
  res.locals.user = req.session.user || null;
  next();
});

app.use('/', publicRoutes);
app.use('/', authRoutes);

// 尚未更換預設密碼者，一律先導到變更密碼頁
app.use('/admin', requirePasswordChanged);
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

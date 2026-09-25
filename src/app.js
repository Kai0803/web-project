require('../src/utils/seed');

const path = require('path');
const express = require('express');
const session = require('express-session');
const helmet = require('helmet');

const db = require('./db');
const SqliteStore = require('./middleware/sessionStore');
const csrf = require('./middleware/csrf');
const { upload } = require('./middleware/upload');
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

// 簽名圖以 data URL 送出，需要比預設 100kb 更寬的上限
app.use(express.urlencoded({ extended: true, limit: '3mb' }));
app.use(express.static(path.join(__dirname, '..', 'public')));

// 照片上傳為 multipart，body 必須先由 multer 解析，
// 後面的 CSRF 中介層才讀得到 _csrf 欄位。其餘路徑維持不解析 multipart，
// 這樣任何非預期的 multipart 請求都會因缺少 token 而被 CSRF 擋下。
const photoUpload = upload.array('photos', 10);
app.use((req, res, next) => {
  if (req.method === 'POST' && /^\/admin\/orders\/\d+\/photos$/.test(req.path)) {
    return photoUpload(req, res, (err) => {
      if (err) {
        req.uploadError =
          err.code === 'LIMIT_FILE_SIZE' ? '圖片超過 8MB 上限' : err.message || '圖片上傳失敗';
      }
      next();
    });
  }
  next();
});

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

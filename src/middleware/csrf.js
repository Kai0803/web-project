const crypto = require('crypto');

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS']);

/**
 * Session 綁定的 CSRF token。每個 session 一組 token，
 * 所有會改變資料的請求（POST 等）都必須帶上相符的 _csrf 欄位。
 */
function csrf(req, res, next) {
  if (!req.session.csrfToken) {
    req.session.csrfToken = crypto.randomBytes(32).toString('hex');
  }
  res.locals.csrfToken = req.session.csrfToken;

  if (SAFE_METHODS.has(req.method)) return next();

  const submitted = (req.body && req.body._csrf) || req.get('x-csrf-token');
  const expected = req.session.csrfToken;

  const valid =
    typeof submitted === 'string' &&
    submitted.length === expected.length &&
    crypto.timingSafeEqual(Buffer.from(submitted), Buffer.from(expected));

  if (!valid) {
    return res.status(403).render('error', {
      title: '請求已失效',
      message: '安全驗證失敗，可能是頁面停留過久。請回上一頁重新整理後再送出一次。',
      user: req.session.user || null,
    });
  }

  next();
}

module.exports = csrf;

function requireAuth(req, res, next) {
  if (!req.session.user) {
    req.session.returnTo = req.originalUrl;
    return res.redirect('/login');
  }
  next();
}

function requireRole(...roles) {
  return (req, res, next) => {
    if (!req.session.user || !roles.includes(req.session.user.role)) {
      return res.status(403).render('error', {
        title: '權限不足',
        message: '您沒有權限存取此頁面。',
        user: req.session.user || null,
      });
    }
    next();
  };
}

function requirePasswordChanged(req, res, next) {
  if (req.session.user && req.session.user.mustChangePassword) {
    return res.redirect('/change-password');
  }
  next();
}

module.exports = { requireAuth, requireRole, requirePasswordChanged };

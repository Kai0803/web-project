const STATUS_LABELS = {
  pending: '待確認',
  scheduled: '已排程',
  in_progress: '服務中',
  completed: '已完成',
  cancelled: '已取消',
};

const STATUS_BADGE_CLASS = {
  pending: 'badge-pending',
  scheduled: 'badge-scheduled',
  in_progress: 'badge-progress',
  completed: 'badge-completed',
  cancelled: 'badge-cancelled',
};

const TOOL_STATUS_LABELS = {
  available: '可借用',
  in_use: '使用中',
  maintenance: '維修中',
  retired: '已報廢',
};

function generateBookingCode(db) {
  const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  for (let attempt = 0; attempt < 10; attempt++) {
    let code = 'B';
    for (let i = 0; i < 6; i++) {
      code += chars[Math.floor(Math.random() * chars.length)];
    }
    const exists = db.prepare('SELECT 1 FROM orders WHERE booking_code = ?').get(code);
    if (!exists) return code;
  }
  throw new Error('無法產生唯一預約編號，請重試');
}

function formatMoney(n) {
  return 'NT$ ' + Number(n || 0).toLocaleString('zh-TW');
}

module.exports = {
  STATUS_LABELS,
  STATUS_BADGE_CLASS,
  TOOL_STATUS_LABELS,
  generateBookingCode,
  formatMoney,
};

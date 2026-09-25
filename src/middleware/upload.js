const path = require('path');
const fs = require('fs');
const crypto = require('crypto');
const multer = require('multer');

// 照片放在 data/ 之下，與資料庫同一個 volume，容器重建不會遺失
const uploadDir = path.join(__dirname, '..', '..', 'data', 'uploads');
if (!fs.existsSync(uploadDir)) fs.mkdirSync(uploadDir, { recursive: true });

const ALLOWED = {
  'image/jpeg': '.jpg',
  'image/png': '.png',
  'image/webp': '.webp',
};

const storage = multer.diskStorage({
  destination: (req, file, cb) => cb(null, uploadDir),
  filename: (req, file, cb) => {
    // 不沿用使用者提供的檔名，避免路徑穿越與覆寫
    cb(null, crypto.randomBytes(16).toString('hex') + ALLOWED[file.mimetype]);
  },
});

const upload = multer({
  storage,
  limits: { fileSize: 8 * 1024 * 1024, files: 10 },
  fileFilter: (req, file, cb) => {
    if (!ALLOWED[file.mimetype]) return cb(new Error('僅支援 JPG / PNG / WebP 圖片'));
    cb(null, true);
  },
});

/** 將簽名 canvas 的 data URL 寫成 PNG 檔，回傳檔名 */
function saveSignature(dataUrl) {
  const match = /^data:image\/png;base64,([A-Za-z0-9+/=]+)$/.exec(dataUrl || '');
  if (!match) return null;

  const buffer = Buffer.from(match[1], 'base64');
  if (buffer.length > 2 * 1024 * 1024) return null;

  const filename = crypto.randomBytes(16).toString('hex') + '.png';
  fs.writeFileSync(path.join(uploadDir, filename), buffer);
  return filename;
}

function photoPath(filename) {
  // 僅接受本模組產生的檔名格式，杜絕路徑穿越
  if (!/^[0-9a-f]{32}\.(jpg|png|webp)$/.test(filename)) return null;
  return path.join(uploadDir, filename);
}

function removePhoto(filename) {
  const target = photoPath(filename);
  if (target && fs.existsSync(target)) fs.unlinkSync(target);
}

module.exports = { upload, uploadDir, saveSignature, photoPath, removePhoto };

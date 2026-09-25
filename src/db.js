const path = require('path');
const fs = require('fs');
const Database = require('better-sqlite3');

const dataDir = path.join(__dirname, '..', 'data');
if (!fs.existsSync(dataDir)) fs.mkdirSync(dataDir, { recursive: true });

const db = new Database(path.join(dataDir, 'app.sqlite'));
db.pragma('journal_mode = WAL');
db.pragma('foreign_keys = ON');

db.exec(`
CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('admin','engineer')),
  phone TEXT,
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS service_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  category TEXT NOT NULL DEFAULT '其他',
  unit TEXT NOT NULL DEFAULT '次',
  base_price REAL NOT NULL DEFAULT 0,
  est_minutes INTEGER NOT NULL DEFAULT 60,
  is_public INTEGER NOT NULL DEFAULT 1,
  note TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS parts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  spec TEXT,
  unit TEXT NOT NULL DEFAULT '個',
  stock_qty INTEGER NOT NULL DEFAULT 0,
  unit_price REAL NOT NULL DEFAULT 0,
  low_stock_threshold INTEGER NOT NULL DEFAULT 5,
  note TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tools (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  code TEXT NOT NULL UNIQUE,
  category TEXT,
  status TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available','in_use','maintenance','retired')),
  current_holder_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  note TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tool_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  tool_id INTEGER NOT NULL REFERENCES tools(id) ON DELETE CASCADE,
  engineer_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  order_id INTEGER REFERENCES orders(id) ON DELETE SET NULL,
  action TEXT NOT NULL CHECK (action IN ('checkout','return','maintenance','retired','available')),
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS orders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  booking_code TEXT NOT NULL UNIQUE,
  customer_name TEXT NOT NULL,
  phone TEXT NOT NULL,
  address TEXT NOT NULL,
  preferred_date TEXT,
  preferred_time TEXT,
  note TEXT,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','scheduled','in_progress','completed','cancelled')),
  assigned_engineer_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  scheduled_date TEXT,
  scheduled_time TEXT,
  total_amount REAL NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS order_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  service_item_id INTEGER REFERENCES service_items(id) ON DELETE SET NULL,
  name_snapshot TEXT NOT NULL,
  qty INTEGER NOT NULL DEFAULT 1,
  unit_price REAL NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS order_parts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  part_id INTEGER REFERENCES parts(id) ON DELETE SET NULL,
  name_snapshot TEXT NOT NULL,
  qty INTEGER NOT NULL DEFAULT 1,
  unit_price REAL NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_orders_phone ON orders(phone);
CREATE INDEX IF NOT EXISTS idx_orders_engineer ON orders(assigned_engineer_id);

CREATE TABLE IF NOT EXISTS order_photos (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  filename TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'other' CHECK (kind IN ('before','after','other','signature')),
  caption TEXT,
  uploaded_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_order_photos_order ON order_photos(order_id);

CREATE TABLE IF NOT EXISTS sessions (
  sid TEXT PRIMARY KEY,
  expires_at INTEGER NOT NULL,
  data TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);
`);

// 既有資料庫的欄位增補（CREATE TABLE IF NOT EXISTS 不會補欄位）
function addColumnIfMissing(table, column, definition) {
  const columns = db.prepare(`PRAGMA table_info(${table})`).all();
  if (!columns.some((c) => c.name === column)) {
    db.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${definition}`);
  }
}

addColumnIfMissing('orders', 'completed_at', 'TEXT');
addColumnIfMissing('users', 'must_change_password', 'INTEGER NOT NULL DEFAULT 0');
addColumnIfMissing('users', 'failed_attempts', 'INTEGER NOT NULL DEFAULT 0');
addColumnIfMissing('users', 'locked_until', 'TEXT');

module.exports = db;

const bcrypt = require('bcryptjs');
const db = require('../db');

function seed() {
  const userCount = db.prepare('SELECT COUNT(*) AS c FROM users').get().c;
  if (userCount === 0) {
    const insertUser = db.prepare(`
      INSERT INTO users (name, username, password_hash, role, phone)
      VALUES (@name, @username, @password_hash, @role, @phone)
    `);
    insertUser.run({
      name: '系統管理員',
      username: 'admin',
      password_hash: bcrypt.hashSync('admin1234', 10),
      role: 'admin',
      phone: '0900-000-000',
    });
    insertUser.run({
      name: '王師傅',
      username: 'engineer1',
      password_hash: bcrypt.hashSync('engineer1234', 10),
      role: 'engineer',
      phone: '0900-111-111',
    });
    console.log('已建立預設帳號：admin / admin1234，engineer1 / engineer1234');
  }

  const serviceCount = db.prepare('SELECT COUNT(*) AS c FROM service_items').get().c;
  if (serviceCount === 0) {
    const insertService = db.prepare(`
      INSERT INTO service_items (name, category, unit, base_price, est_minutes, is_public, note)
      VALUES (@name, @category, @unit, @base_price, @est_minutes, @is_public, @note)
    `);
    const items = [
      { name: '分離式冷氣清洗（一般型）', category: '清洗保養', unit: '台', base_price: 800, est_minutes: 60, is_public: 1, note: '含濾網、面板清潔' },
      { name: '分離式冷氣清洗（變頻高壓型）', category: '清洗保養', unit: '台', base_price: 1500, est_minutes: 90, is_public: 1, note: '含高壓水洗、排水管檢測' },
      { name: '窗型冷氣清洗', category: '清洗保養', unit: '台', base_price: 900, est_minutes: 60, is_public: 1, note: '' },
      { name: '冷氣不冷檢修', category: '維修', unit: '次', base_price: 500, est_minutes: 60, is_public: 1, note: '基本檢修費，另計零件費' },
      { name: '冷媒填充', category: '維修', unit: '次', base_price: 1200, est_minutes: 60, is_public: 1, note: '依冷媒種類與用量另計' },
      { name: '主機板 / 電控維修', category: '維修', unit: '次', base_price: 800, est_minutes: 90, is_public: 1, note: '' },
      { name: '分離式冷氣安裝（一對一）', category: '安裝', unit: '台', base_price: 3500, est_minutes: 180, is_public: 1, note: '含基本管線 3 米' },
      { name: '移機（含拆裝）', category: '安裝', unit: '台', base_price: 2500, est_minutes: 150, is_public: 1, note: '' },
      { name: '室外機支架加固', category: '安裝', unit: '次', base_price: 600, est_minutes: 40, is_public: 0, note: '內部估價項目' },
    ];
    const insertMany = db.transaction((rows) => rows.forEach((r) => insertService.run(r)));
    insertMany(items);
    console.log('已建立預設服務項目（工項）');
  }

  const partsCount = db.prepare('SELECT COUNT(*) AS c FROM parts').get().c;
  if (partsCount === 0) {
    const insertPart = db.prepare(`
      INSERT INTO parts (name, spec, unit, stock_qty, unit_price, low_stock_threshold, note)
      VALUES (@name, @spec, @unit, @stock_qty, @unit_price, @low_stock_threshold, @note)
    `);
    const parts = [
      { name: 'R32 冷媒', spec: '瓶裝 800g', unit: '瓶', stock_qty: 12, unit_price: 900, low_stock_threshold: 3, note: '' },
      { name: 'R410A 冷媒', spec: '瓶裝 800g', unit: '瓶', stock_qty: 8, unit_price: 950, low_stock_threshold: 3, note: '' },
      { name: '銅管 1/4"', spec: '每米', unit: '米', stock_qty: 60, unit_price: 60, low_stock_threshold: 10, note: '' },
      { name: '銅管 3/8"', spec: '每米', unit: '米', stock_qty: 60, unit_price: 90, low_stock_threshold: 10, note: '' },
      { name: '保溫材', spec: '9mm 厚', unit: '米', stock_qty: 40, unit_price: 30, low_stock_threshold: 10, note: '' },
      { name: '排水管', spec: '標準型', unit: '米', stock_qty: 50, unit_price: 25, low_stock_threshold: 10, note: '' },
      { name: '電容器', spec: '35uF', unit: '顆', stock_qty: 15, unit_price: 250, low_stock_threshold: 5, note: '' },
      { name: '主機板通用型', spec: '變頻用', unit: '片', stock_qty: 4, unit_price: 1800, low_stock_threshold: 2, note: '' },
    ];
    const insertMany = db.transaction((rows) => rows.forEach((r) => insertPart.run(r)));
    insertMany(parts);
    console.log('已建立預設零件庫存');
  }

  const toolsCount = db.prepare('SELECT COUNT(*) AS c FROM tools').get().c;
  if (toolsCount === 0) {
    const insertTool = db.prepare(`
      INSERT INTO tools (name, code, category, status, note)
      VALUES (@name, @code, @category, @status, @note)
    `);
    const tools = [
      { name: '高壓清洗機', code: 'TL-001', category: '清洗設備', status: 'available', note: '' },
      { name: '冷媒回收機', code: 'TL-002', category: '維修設備', status: 'available', note: '' },
      { name: '真空幫浦', code: 'TL-003', category: '安裝設備', status: 'available', note: '' },
      { name: '電子秤（冷媒填充用）', code: 'TL-004', category: '維修設備', status: 'available', note: '' },
      { name: '電鑽組', code: 'TL-005', category: '安裝設備', status: 'maintenance', note: '電池待更換' },
      { name: '三用電表', code: 'TL-006', category: '維修設備', status: 'available', note: '' },
    ];
    const insertMany = db.transaction((rows) => rows.forEach((r) => insertTool.run(r)));
    insertMany(tools);
    console.log('已建立預設工具清單');
  }
}

seed();

if (require.main === module) {
  console.log('資料初始化完成。');
}

module.exports = seed;

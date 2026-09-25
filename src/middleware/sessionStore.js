const session = require('express-session');
const db = require('../db');

/**
 * 以現有的 better-sqlite3 連線實作 session store，
 * 避免為了 connect-sqlite3 再引入一個原生模組。
 * session 存進資料庫後，重啟服務不會把所有人登出。
 */
class SqliteStore extends session.Store {
  constructor() {
    super();
    this.getStmt = db.prepare('SELECT data, expires_at FROM sessions WHERE sid = ?');
    this.setStmt = db.prepare(`
      INSERT INTO sessions (sid, expires_at, data) VALUES (@sid, @expires_at, @data)
      ON CONFLICT(sid) DO UPDATE SET expires_at = @expires_at, data = @data
    `);
    this.destroyStmt = db.prepare('DELETE FROM sessions WHERE sid = ?');
    this.touchStmt = db.prepare('UPDATE sessions SET expires_at = ? WHERE sid = ?');
    this.cleanupStmt = db.prepare('DELETE FROM sessions WHERE expires_at < ?');

    // 每小時清掉過期 session
    this.cleanupTimer = setInterval(() => this.cleanup(), 1000 * 60 * 60);
    this.cleanupTimer.unref();
    this.cleanup();
  }

  cleanup() {
    try {
      this.cleanupStmt.run(Date.now());
    } catch (err) {
      // 清理失敗不影響服務運作
    }
  }

  expiryOf(sess) {
    const maxAge = sess && sess.cookie && sess.cookie.maxAge;
    return Date.now() + (maxAge || 1000 * 60 * 60 * 8);
  }

  get(sid, callback) {
    try {
      const row = this.getStmt.get(sid);
      if (!row) return callback(null, null);
      if (row.expires_at < Date.now()) {
        this.destroyStmt.run(sid);
        return callback(null, null);
      }
      callback(null, JSON.parse(row.data));
    } catch (err) {
      callback(err);
    }
  }

  set(sid, sess, callback) {
    try {
      this.setStmt.run({
        sid,
        expires_at: this.expiryOf(sess),
        data: JSON.stringify(sess),
      });
      callback(null);
    } catch (err) {
      callback(err);
    }
  }

  destroy(sid, callback) {
    try {
      this.destroyStmt.run(sid);
      callback(null);
    } catch (err) {
      callback(err);
    }
  }

  touch(sid, sess, callback) {
    try {
      this.touchStmt.run(this.expiryOf(sess), sid);
      callback(null);
    } catch (err) {
      callback(err);
    }
  }
}

module.exports = SqliteStore;

-- sessions.id 改存 token 的 SHA-256 摘要，不再存 token 本身。
--
-- 原本那一欄就是 bearer 憑證：能讀資料庫或拿到 pg_dump 備份的人
-- 可以直接把值貼進 cookie 冒充任何使用者，而稽核會記成受害者。
--
-- 既有列的格式不相容（存的是原始 token），直接清除。第一階段尚未上線，
-- 代價只是全員重新登入一次。

DELETE FROM sessions;

COMMENT ON COLUMN sessions.id IS
  'session token 的 SHA-256 hex 摘要，非 token 本身。token 只存在於使用者的 cookie。';

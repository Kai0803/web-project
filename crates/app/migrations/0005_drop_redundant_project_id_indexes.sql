-- quotations(project_id) 與 contracts(project_id) 的單欄索引是冗餘的。
--
-- 兩者都被以 project_id 為首欄的複合 UNIQUE 完全涵蓋：
--   quotations_project_id_id_key (project_id, id)   -- 0001，供 contracts 的複合外鍵
--   contracts_project_id_id_key  (project_id, id)   -- 0004，供 work_items 的複合外鍵
-- 任何能用單欄索引的查詢都能用複合索引的前綴。
--
-- contracts 那一個是 0004 加入複合 UNIQUE 之後才變成冗餘的，不是 0001 寫錯。
--
-- 實測（1,500 專案 / 7,500 報價）DROP 之後沒有任何查詢變慢：
-- list_for_project 從 0.033 ms 變 0.022 ms（改走複合索引），
-- count_for_project 變成 Index Only Scan。
--
-- 刪掉的理由不是那 1 µs 的寫入成本，是少一份會讓人誤以為有用的重複索引。

DROP INDEX quotations_project_id_idx;
DROP INDEX contracts_project_id_idx;

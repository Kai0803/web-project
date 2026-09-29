<!-- 原由 agent_tools 從 src/skills/dev-workflow/references/dsa-checklist.md 產生，
     已於本 repo 手動改寫為 Rust / TypeScript 版。修改後請同步回源頭，否則下次產生會被覆蓋。 -->
# Data Structures & Algorithms Checklist (Rust + TypeScript)

聚焦「選對容器了嗎、複雜度可接受嗎、熱點在哪」。不做微觀調參。

## A. 複雜度標註
- 每個核心函式標出時間與空間複雜度。
- 複雜度以 G1 的預期資料量代入，估出實際規模（n=10⁴ 與 n=10⁷ 結論可能相反）。
- 巢狀迴圈已確認不是 O(n²) 掃描，或已確認 n 夠小。
- 遞迴有深度評估：Rust 無遞迴上限，過深直接 stack overflow（預設主執行緒約 8MB）。資料驅動的遞迴改為顯式堆疊。
- **資料庫查詢的複雜度優先於記憶體內演算法**：一個 N+1 查詢的代價遠大於任何容器選型。

## B. 容器選型（Rust）
- 成員查詢用 `HashSet` / `HashMap`（O(1)），不用 `Vec::contains`（O(n)）。
- 頭部插入或彈出用 `VecDeque`，不用 `Vec::remove(0)` / `Vec::insert(0, _)`（O(n)）。
- 需要有序走訪或範圍查詢用 `BTreeMap` / `BTreeSet`。
- 已知筆數時用 `Vec::with_capacity` / `HashMap::with_capacity`，避免反覆重新配置。
- 需要 top-k 用 `BinaryHeap`，不做全排序。
- 固定小結構用 tuple 或 struct，不用 `HashMap` 傳結構化資料。
- 鍵為小型整數且密集時，`Vec` 索引比 `HashMap` 更快也更省。
- 大量數值運算考慮 `ndarray`；一般業務邏輯不需要。

## C. 迴圈與重複計算
- 迴圈內沒有可提到迴圈外的不變計算；regex 以 `once_cell::Lazy` / `LazyLock` 編譯一次。
- 字串累加用 `String::push_str` 搭配 `with_capacity`，或 `join`，不在迴圈內 `format!`。
- 迴圈內沒有重複建立相同的集合、查詢資料庫、或重新準備 SQL statement。
- 優先使用 iterator adaptor 鏈；避免中途 `collect()` 成臨時 `Vec` 再繼續處理。
- 重複的純函式呼叫可考慮快取，但需確認輸入可 hash 且結果穩定。

## D. 記憶體與所有權（Rust 重點）
- 每個 `.clone()` 都是有意識的決定，不是為了讓借用檢查器通過。
- 能借用就借用：函式參數用 `&str` / `&[T]`，回傳才考慮擁有型別。
- 大資料集用 iterator 串流處理，不一次 `collect()` 成完整 `Vec`。
- 單執行緒共享用 `Rc`，跨執行緒才用 `Arc`；不要習慣性一律 `Arc`。
- 讀檔用 `BufReader` 逐行迭代，不用 `read_to_string` 全載入。
- 長生命週期結構不持有大型暫時資料的參考或複本。
- 注意 `String` 與 `Vec` 的容量不會自動縮減；長期持有可考慮 `shrink_to_fit`。

## E. 演算法選擇
- 問題是否對應已知模式：排序、雙指標、滑動視窗、前綴和、雜湊映射、圖走訪、動態規劃。
- 有沒有標準庫或 `itertools` 的現成解法，而不需自行實作。
- 排序用 `sort_by_key` / `sort_unstable_by_key`（不需穩定性時後者較快），不自行實作比較排序。
- 已排序資料的查找用 `binary_search`，不做線性掃描。
- 需要多次查詢時，考慮一次預處理換取後續 O(1) / O(log n) 查詢。
- 若選擇較差複雜度的做法，說明理由（資料量小、可讀性優先、常數因子）。

## F. 熱點識別
- 指出成本最高的前三個位置，並說明依據。
- 區分「演算法問題」「I/O 問題」「常數因子問題」；前兩者優先。
- **效能量測一律在 `--release` 下進行**：debug build 可能慢 10～100 倍，據此下的結論不可信。
- 建議以 `criterion` 做 benchmark、`cargo flamegraph` 找熱點，不憑感覺優化。
- 優化前記錄基準數據，優化後可比較。
- 前端以瀏覽器 Performance profiler 與 bundle 分析為準。

## G. 正確性交叉檢查
- 優化後的實作與原實作在邊界輸入下結果一致（空、單一、重複元素、極值）。
- 整數運算確認不會溢位；release 模式會靜默 wrap。
- 浮點累加注意誤差累積；金額一律不用浮點。
- 改用 `sort_unstable` 前確認不依賴相等元素的原始順序。
- 平行化（`rayon`）前確認無共享可變狀態，且資料量大到足以攤平執行緒開銷。

## 輸出要求
- Findings 依 Blocker / High / Medium / Low / Nit 排序。
- 每筆含：檔案位置、目前複雜度、建議做法、改善後複雜度、適用前提。
- 明確標示哪些建議需要實測驗證才值得做。
- 若現況已足夠，直接說「無需優化」並附理由，不硬湊建議。

//! typst 的 `World` 實作。
//!
//! typst 預期自己能讀檔案系統與字型庫。這裡提供一個最小的實作：
//! 唯一的來源檔是編譯期內嵌的模板，字型在啟動時載入一次。

use std::sync::OnceLock;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Dict, Duration, IntoValue};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// 模板。編譯期內嵌，不在執行時讀檔——映像裡少一個可能被改動的檔案。
const TEMPLATE: &str = include_str!("quotation.typ");

/// 已載入的字型。掃描成本不低，整個行程只做一次。
static FONTS: OnceLock<LoadedFonts> = OnceLock::new();

struct LoadedFonts {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    /// 實際可用的中文字型名稱，依序交給模板當 fallback 清單。
    cjk_families: Vec<String>,
}

/// 掃描字型目錄並載入。
///
/// 目錄來自 `FONT_DIR`（容器內指向 Noto CJK 的安裝位置）。
/// 另外一律帶上 typst 自己的西文字型，數字與英數字才有像樣的字面。
fn load_fonts() -> &'static LoadedFonts {
    FONTS.get_or_init(|| {
        let mut fonts: Vec<Font> = Vec::new();

        // typst 內附的西文字型
        for data in typst_assets::fonts() {
            let bytes = Bytes::new(data);
            for index in 0..ttf_parser::fonts_in_collection(&bytes).unwrap_or(1) {
                if let Some(font) = Font::new(bytes.clone(), index) {
                    fonts.push(font);
                }
            }
        }

        let dir = std::env::var("FONT_DIR").unwrap_or_else(|_| "/usr/share/fonts".to_owned());
        let before = fonts.len();
        collect_fonts(std::path::Path::new(&dir), &mut fonts, 0);

        let book = FontBook::from_fonts(&fonts);

        // 找出涵蓋中文的字型家族。沒有的話 PDF 會整片豆腐方塊，
        // 與其靜默產出廢紙，不如讓產生時明確失敗。
        // U+6E2C「測」：任何堪用的繁體字型都該有。
        const PROBE: u32 = '測' as u32;
        let mut cjk_families: Vec<String> = Vec::new();
        for font in fonts.iter().skip(before) {
            let info = font.info();
            if info.coverage.contains(PROBE) && !cjk_families.contains(&info.family) {
                cjk_families.push(info.family.clone());
            }
        }

        LoadedFonts {
            book: LazyHash::new(book),
            fonts,
            cjk_families,
        }
    })
}

/// 遞迴蒐集字型檔。深度設限，避免誤指到一個很深的目錄時掃到天荒地老。
fn collect_fonts(dir: &std::path::Path, out: &mut Vec<Font>, depth: usize) {
    const MAX_DEPTH: usize = 4;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, out, depth + 1);
            continue;
        }

        let is_font = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "ttf" | "otf" | "ttc" | "otc"
                )
            })
            .unwrap_or(false);
        if !is_font {
            continue;
        }

        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let bytes = Bytes::new(data);
        for index in 0..ttf_parser::fonts_in_collection(&bytes).unwrap_or(1) {
            if let Some(font) = Font::new(bytes.clone(), index) {
                out.push(font);
            }
        }
    }
}

/// 可用的中文字型家族，供模板設定 fallback 順序。空的代表環境缺中文字型。
pub fn cjk_families() -> &'static [String] {
    &load_fonts().cjk_families
}

pub struct QuotationWorld {
    library: LazyHash<Library>,
    source: Source,
}

impl QuotationWorld {
    /// `data` 為注入模板的 JSON 字串。
    pub fn new(data: String) -> Self {
        let mut inputs = Dict::new();
        inputs.insert("data".into(), data.into_value());

        let library = Library::builder().with_inputs(inputs).build();
        let vpath = VirtualPath::new("quotation.typ").expect("模板路徑為常數，必定合法");
        let id = FileId::new(RootedPath::new(VirtualRoot::Project, vpath));

        Self {
            library: LazyHash::new(library),
            source: Source::new(id, TEMPLATE.to_owned()),
        }
    }
}

impl World for QuotationWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &load_fonts().book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().get_without_slash().into()))
        }
    }

    /// 模板不讀任何外部檔案——logo 與條款都由 inputs 帶入。
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(id.vpath().get_without_slash().into()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        load_fonts().fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        let now = time::OffsetDateTime::now_utc();
        Datetime::from_ymd(now.year(), now.month() as u8, now.day())
    }
}

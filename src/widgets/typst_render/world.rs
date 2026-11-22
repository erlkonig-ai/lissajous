use typst::diag::{FileResult, Severity};
use typst::foundations::{Bytes, Datetime};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// A resolved Typst diagnostic with byte positions and text.
pub struct TypstDiag {
    /// Diagnostic severity (error or warning).
    pub severity: Severity,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Additional hint strings.
    pub hints: Vec<String>,
    /// Byte range in the full source (including preamble).
    pub span_range: Option<std::ops::Range<usize>>,
}

/// Bytes a file resolver may return for a path it recognises but cannot
/// serve right now (not in scope, not yet synced, too large). The painter
/// draws a labelled box in its place instead of these transparent pixels, so
/// one missing picture never turns a whole page into a compile error.
pub const UNAVAILABLE_IMAGE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/typst/unavailable-image.png"
));

/// Caller-supplied bytes for files a document names, for example
/// `image("files:3fa4…")`. It receives the path relative to the document
/// root exactly as Typst resolved it (`files:3fa4…`). GORBIE itself reads
/// nothing from disk or network: without a resolver, and for every path the
/// resolver declines, the file is simply not found.
pub type FileResolver<'a> = dyn Fn(&str) -> FileResult<Bytes> + Send + Sync + 'a;

const MATH_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/NewComputerModern/NewCMMath-Regular.otf"
));

const BODY_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/IosevkaGorbie/IosevkaGorbie-Regular.ttf"
));

const BODY_FONT_BOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/IosevkaGorbie/IosevkaGorbie-Bold.ttf"
));

/// Minimal Typst World for compiling markup in GORBIE.
///
/// Embeds the math font (New Computer Modern Math) and IosevkaGorbie.
/// Keeps a persistent `Source` that can be swapped for incremental
/// compilation via comemo.
pub struct GorbieWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    main_id: FileId,
    source: Source,
    fonts: Vec<Font>,
}

impl GorbieWorld {
    /// Creates a new world with embedded fonts and an empty source.
    pub fn new() -> Self {
        let mut book = FontBook::new();
        let mut fonts = Vec::new();

        // Load embedded fonts.
        for data in [MATH_FONT, BODY_FONT, BODY_FONT_BOLD] {
            let bytes = Bytes::new(data);
            for font in Font::iter(bytes) {
                book.push(font.info().clone());
                fonts.push(font);
            }
        }

        let main_id = FileId::new(None, VirtualPath::new("main.typ"));
        let source = Source::new(main_id, String::new());

        Self {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(book),
            main_id,
            source,
            fonts,
        }
    }

    /// Access the current source (for span resolution during copy).
    pub fn main_source(&self) -> &Source {
        &self.source
    }

    /// Set the source content for the next compilation.
    /// Uses `Source::replace` for incremental reparsing — only the
    /// changed portion of the syntax tree is rebuilt.
    pub fn set_source(&mut self, text: String) {
        self.source.replace(&text);
    }

    /// Compile the current source into a paged document.
    pub fn compile(&self) -> Result<PagedDocument, Vec<TypstDiag>> {
        self.diagnose(typst::compile::<PagedDocument>(self).output)
    }

    /// Compile with `files` answering every file the document names.
    ///
    /// The resolver is borrowed for this one compilation only, so a caller
    /// can scope it to whatever data it is rendering from this frame.
    pub fn compile_with_files(
        &self,
        files: &FileResolver<'_>,
    ) -> Result<PagedDocument, Vec<TypstDiag>> {
        let world = ResolvingWorld { base: self, files };
        self.diagnose(typst::compile::<PagedDocument>(&world).output)
    }

    fn diagnose(
        &self,
        output: typst::diag::SourceResult<PagedDocument>,
    ) -> Result<PagedDocument, Vec<TypstDiag>> {
        output.map_err(|errors| {
            errors
                .iter()
                .map(|e| TypstDiag {
                    severity: e.severity,
                    message: e.message.to_string(),
                    hints: e.hints.iter().map(|h| h.to_string()).collect(),
                    span_range: self.source.range(e.span),
                })
                .collect()
        })
    }
}

fn not_found(id: FileId) -> typst::diag::FileError {
    typst::diag::FileError::NotFound(id.vpath().as_rootless_path().into())
}

/// The base world with one compilation's file resolver beside it.
struct ResolvingWorld<'a> {
    base: &'a GorbieWorld,
    files: &'a FileResolver<'a>,
}

impl World for ResolvingWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        self.base.library()
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.base.book()
    }

    fn main(&self) -> FileId {
        self.base.main()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.base.source(id)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        // Packages are never resolved; a path that is not valid UTF-8 cannot
        // name anything a resolver knows.
        if id.package().is_some() {
            return Err(not_found(id));
        }
        match id.vpath().as_rootless_path().to_str() {
            Some(path) => (self.files)(path),
            None => Err(not_found(id)),
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.base.font(index)
    }

    fn today(&self, offset: Option<i64>) -> Option<Datetime> {
        self.base.today(offset)
    }
}

impl World for GorbieWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.source.clone())
        } else {
            Err(typst::diag::FileError::NotFound(
                id.vpath().as_rootless_path().into(),
            ))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(not_found(id))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

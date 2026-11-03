//! Which file formats exist, and which views each one offers.

use std::path::Path;

/// How a document's content can be rendered besides plain source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Code,
    Markdown,
    Latex,
    Json,
    Yaml,
    Toml,
    Xml,
    Csv,
    Tsv,
    Spreadsheet,
    Image,
}

/// What the editor area shows for one document.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    Source,
    Split,
    Preview,
}

impl Format {
    pub fn detect(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "md" | "markdown" | "mdx" | "mdown" | "mkd" | "rmd" | "qmd" => Self::Markdown,
            "tex" | "latex" | "ltx" | "sty" | "cls" | "katex" => Self::Latex,
            "json" | "jsonc" | "json5" | "geojson" | "ipynb" | "webmanifest" | "har" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "xml" | "xsl" | "xslt" | "xsd" | "svg" | "plist" | "xaml" | "csproj" | "fsproj"
            | "vcxproj" | "rss" | "atom" | "kml" | "gpx" | "pom" | "storyboard" | "xib" => Self::Xml,
            "csv" => Self::Csv,
            "tsv" | "tab" => Self::Tsv,
            "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => Self::Spreadsheet,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => Self::Image,
            _ => Self::Code,
        }
    }

    /// Binary formats are never shown as editable text.
    pub fn is_binary(self) -> bool {
        matches!(self, Self::Spreadsheet | Self::Image)
    }

    pub fn has_preview(self) -> bool {
        !matches!(self, Self::Code)
    }

    /// The mode a freshly opened document starts in.
    pub fn default_view(self) -> ViewMode {
        match self {
            Self::Spreadsheet | Self::Image => ViewMode::Preview,
            _ => ViewMode::Source,
        }
    }

    /// Short human label for the preview button ("预览", "树", "表格"…).
    pub fn preview_label(self) -> &'static str {
        match self {
            Self::Markdown | Self::Latex => "预览",
            Self::Json | Self::Yaml | Self::Toml | Self::Xml => "树",
            Self::Csv | Self::Tsv | Self::Spreadsheet => "表格",
            Self::Image => "图像",
            Self::Code => "",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Markdown => "Markdown",
            Self::Latex => "LaTeX",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Xml => "XML",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
            Self::Spreadsheet => "Spreadsheet",
            Self::Image => "Image",
        }
    }
}

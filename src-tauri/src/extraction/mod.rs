pub mod resolver;

use serde::{Deserialize, Serialize};

/// Unified logical screen coordinate (origin at top-left of primary display).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance_to(self, other: Self) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// Unified logical screen rectangle (origin at top-left of primary display).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ScreenRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn min_x(&self) -> f64 {
        self.x
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    pub fn min_y(&self) -> f64 {
        self.y
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }

    pub fn mid_x(&self) -> f64 {
        self.x + self.width * 0.5
    }

    pub fn mid_y(&self) -> f64 {
        self.y + self.height * 0.5
    }

    pub fn is_valid(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.x.is_finite()
            && self.y.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    pub fn contains(&self, pt: ScreenPoint) -> bool {
        pt.x >= self.min_x()
            && pt.x <= self.max_x()
            && pt.y >= self.min_y()
            && pt.y <= self.max_y()
    }

    pub fn inset_by(&self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            width: (self.width - 2.0 * dx).max(0.0),
            height: (self.height - 2.0 * dy).max(0.0),
        }
    }

    pub fn union(&self, other: Self) -> Self {
        if !self.is_valid() {
            return other;
        }
        if !other.is_valid() {
            return *self;
        }
        let min_x = self.min_x().min(other.min_x());
        let min_y = self.min_y().min(other.min_y());
        let max_x = self.max_x().max(other.max_x());
        let max_y = self.max_y().max(other.max_y());
        Self::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.min_x() < other.max_x()
            && self.max_x() > other.min_x()
            && self.min_y() < other.max_y()
            && self.max_y() > other.min_y()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractionScope {
    Word,
    Sentence,
    Selection,
    Label,
    RegionOcr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractionCompleteness {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractionSource {
    Accessibility,
    Ocr,
    ClipboardFallback,
    PrimarySelection,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractionFailure {
    PermissionDenied,
    BlockedSensitive,
    ExcludedApp,
    PositionUnresolved,
    NotSupported,
    NoText,
    TooLong,
}

impl ExtractionFailure {
    pub fn should_fallback_to_ocr(self) -> bool {
        matches!(
            self,
            Self::PositionUnresolved | Self::NotSupported | Self::NoText
        )
    }

    pub fn user_message(self) -> &'static str {
        match self {
            Self::PermissionDenied => "需要授予辅助功能或屏幕录制权限以读取屏幕文字",
            Self::BlockedSensitive => "安全/密码输入框已自动保护跳过",
            Self::ExcludedApp => "当前应用在排除列表中",
            Self::PositionUnresolved | Self::NotSupported | Self::NoText => "未在光标处识别到文字",
            Self::TooLong => "所选文本超出单次翻译长度上限 (5,000 字符)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSnapshot {
    pub generation: u64,
    pub text: String,
    /// Surrounding sentence when hovering a single word; enables contextual sense
    /// disambiguation AND instant one-click `[译整句]` expansion without re-hovering.
    pub context: Option<String>,
    pub sentence_completeness: Option<ExtractionCompleteness>,
    pub scope: ExtractionScope,
    pub completeness: ExtractionCompleteness,
    pub source: ExtractionSource,
    pub anchor_rect: Option<ScreenRect>,
    pub pointer: ScreenPoint,
    pub app_id: Option<String>,
}

pub const MAX_INPUT_CHARS: usize = 5_000;

pub fn is_allowed_input_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= MAX_INPUT_CHARS
}

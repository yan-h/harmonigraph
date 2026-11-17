use crate::Size;

/// The dpi scaling policy of the window
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum WindowScalePolicy {
    /// Use the system's dpi scale factor
    #[default]
    SystemScaleFactor,
    /// Use the given dpi scale factor (e.g. `1.0` = 96 dpi)
    ScaleFactor(f64),
}

/// The options for opening a new window
#[derive(Debug, Clone, PartialEq)]
pub struct WindowOpenOptions {
    pub title: String,

    /// The logical size of the window
    ///
    /// These dimensions will be scaled by the scaling policy specified in `scale`. Mouse
    /// position will be passed back as logical coordinates.
    pub size: Size,

    /// The dpi scaling policy
    pub scale: WindowScalePolicy,
}

impl WindowOpenOptions {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[inline]
    pub fn with_size(mut self, width: f64, height: f64) -> Self {
        self.size = Size::new(width, height);
        self
    }

    #[inline]
    pub fn with_scale_policy(mut self, scale: WindowScalePolicy) -> Self {
        self.scale = scale;
        self
    }
}

impl Default for WindowOpenOptions {
    fn default() -> Self {
        Self {
            title: String::from("baseview window"),
            size: Size { width: 500.0, height: 400.0 },
            scale: WindowScalePolicy::default(),
        }
    }
}

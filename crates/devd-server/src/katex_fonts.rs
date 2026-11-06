//! Embedded KaTeX font files, one match so the binary has no external font path.
pub fn bytes(file: &str) -> Option<&'static [u8]> {
    Some(match file {
        "KaTeX_AMS-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_AMS-Regular.woff2"),
        "KaTeX_Caligraphic-Bold.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2"),
        "KaTeX_Caligraphic-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2"),
        "KaTeX_Fraktur-Bold.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2"),
        "KaTeX_Fraktur-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2"),
        "KaTeX_Main-Bold.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Main-Bold.woff2"),
        "KaTeX_Main-BoldItalic.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2"),
        "KaTeX_Main-Italic.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Main-Italic.woff2"),
        "KaTeX_Main-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Main-Regular.woff2"),
        "KaTeX_Math-BoldItalic.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2"),
        "KaTeX_Math-Italic.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Math-Italic.woff2"),
        "KaTeX_SansSerif-Bold.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2"),
        "KaTeX_SansSerif-Italic.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2"),
        "KaTeX_SansSerif-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2"),
        "KaTeX_Script-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Script-Regular.woff2"),
        "KaTeX_Size1-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Size1-Regular.woff2"),
        "KaTeX_Size2-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Size2-Regular.woff2"),
        "KaTeX_Size3-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Size3-Regular.woff2"),
        "KaTeX_Size4-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Size4-Regular.woff2"),
        "KaTeX_Typewriter-Regular.woff2" => include_bytes!("../ui/vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2"),
        _ => return None,
    })
}


use ratatui::style::Color;

/// Complete UI theme palette covering all TUI components (status bar, panels, modals, inputs, error banners, status indicators).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_inactive_bg: Color,
    pub tab_inactive_fg: Color,
    pub leader_bg: Color,
    pub leader_fg: Color,
    pub status_busy: Color,
    pub status_waiting: Color,
    pub status_done: Color,
    pub status_error: Color,
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub selection_indicator: Color,
    pub modal_title: Color,
    pub modal_label: Color,
    pub input_text: Color,
    pub input_placeholder: Color,
    pub key_hint: Color,
    pub error_banner_bg: Color,
    pub error_banner_fg: Color,
}

impl Theme {
    /// Tokyo Night (Default): Modern deep slate/navy chrome (`#1a1b26`), electric cyan & blue tab highlights (`#7aa2f7`), vibrant purple borders (`#bb9af7`), and warm amber gold leader mode (`#e0af68`).
    pub fn tokyo_night() -> Self {
        Self {
            name: "tokyo-night".to_string(),
            status_bar_bg: Color::Rgb(26, 27, 38),       // #1a1b26
            status_bar_fg: Color::Rgb(169, 177, 214),   // #a9b1d6
            tab_active_bg: Color::Rgb(122, 162, 247),   // #7aa2f7
            tab_active_fg: Color::Rgb(26, 27, 38),       // #1a1b26
            tab_inactive_bg: Color::Rgb(36, 40, 59),     // #24283b
            tab_inactive_fg: Color::Rgb(169, 177, 214), // #a9b1d6
            leader_bg: Color::Rgb(224, 175, 104),       // #e0af68
            leader_fg: Color::Rgb(26, 27, 38),          // #1a1b26
            status_busy: Color::Rgb(224, 175, 104),      // #e0af68
            status_waiting: Color::Rgb(122, 162, 247),   // #7aa2f7
            status_done: Color::Rgb(158, 206, 106),      // #9ece6a
            status_error: Color::Rgb(247, 118, 142),     // #f7768e
            border_focused: Color::Rgb(187, 154, 247),   // #bb9af7
            border_unfocused: Color::Rgb(65, 72, 104),   // #414868
            selection_bg: Color::Rgb(122, 162, 247),     // #7aa2f7
            selection_fg: Color::Rgb(26, 27, 38),       // #1a1b26
            selection_indicator: Color::Rgb(122, 162, 247), // #7aa2f7
            modal_title: Color::Rgb(187, 154, 247),      // #bb9af7
            modal_label: Color::Rgb(122, 162, 247),      // #7aa2f7
            input_text: Color::Rgb(224, 175, 104),       // #e0af68
            input_placeholder: Color::Rgb(86, 95, 137),  // #565f89
            key_hint: Color::Rgb(122, 162, 247),        // #7aa2f7
            error_banner_bg: Color::Rgb(247, 118, 142),  // #f7768e
            error_banner_fg: Color::Rgb(26, 27, 38),     // #1a1b26
        }
    }

    /// Catppuccin Mocha: Soothing dark pastel palette (`#1e1e2e`), mauve borders (`#cba6f7`), sky blue selections (`#89dceb`), peach input highlights (`#fab387`), and crust backgrounds.
    pub fn catppuccin() -> Self {
        Self {
            name: "catppuccin".to_string(),
            status_bar_bg: Color::Rgb(17, 17, 27),       // #11111b (Crust)
            status_bar_fg: Color::Rgb(205, 214, 244),   // #cdd6f4 (Text)
            tab_active_bg: Color::Rgb(137, 220, 235),   // #89dceb (Sky)
            tab_active_fg: Color::Rgb(17, 17, 27),       // #11111b (Crust)
            tab_inactive_bg: Color::Rgb(30, 30, 46),     // #1e1e2e (Base)
            tab_inactive_fg: Color::Rgb(186, 194, 222), // #bac2de (Subtext1)
            leader_bg: Color::Rgb(249, 226, 175),       // #f9e2af (Yellow)
            leader_fg: Color::Rgb(17, 17, 27),          // #11111b
            status_busy: Color::Rgb(249, 226, 175),      // #f9e2af (Yellow)
            status_waiting: Color::Rgb(137, 220, 235),   // #89dceb (Sky)
            status_done: Color::Rgb(166, 227, 161),      // #a6e3a1 (Green)
            status_error: Color::Rgb(243, 139, 168),     // #f38ba8 (Red)
            border_focused: Color::Rgb(203, 166, 247),   // #cba6f7 (Mauve)
            border_unfocused: Color::Rgb(88, 91, 112),   // #585b70 (Surface2)
            selection_bg: Color::Rgb(137, 220, 235),     // #89dceb (Sky)
            selection_fg: Color::Rgb(17, 17, 27),       // #11111b
            selection_indicator: Color::Rgb(137, 220, 235), // #89dceb
            modal_title: Color::Rgb(203, 166, 247),      // #cba6f7 (Mauve)
            modal_label: Color::Rgb(137, 220, 235),      // #89dceb (Sky)
            input_text: Color::Rgb(250, 179, 135),       // #fab387 (Peach)
            input_placeholder: Color::Rgb(108, 112, 134), // #6c7086 (Overlay0)
            key_hint: Color::Rgb(137, 220, 235),        // #89dceb (Sky)
            error_banner_bg: Color::Rgb(243, 139, 168),  // #f38ba8 (Red)
            error_banner_fg: Color::Rgb(17, 17, 27),     // #11111b
        }
    }

    /// Cyberpunk / Synthwave: High-contrast dark cyber aesthetic (`#0a0a0f`), hot neon magenta (`#ff007f`), electric neon cyan (`#00f0ff`), and vivid acid yellow leader indicator (`#ffe600`).
    pub fn cyberpunk() -> Self {
        Self {
            name: "cyberpunk".to_string(),
            status_bar_bg: Color::Rgb(10, 10, 15),       // #0a0a0f
            status_bar_fg: Color::Rgb(240, 240, 245),   // #f0f0f5
            tab_active_bg: Color::Rgb(0, 240, 255),      // #00f0ff (Neon Cyan)
            tab_active_fg: Color::Rgb(10, 10, 15),       // #0a0a0f
            tab_inactive_bg: Color::Rgb(24, 24, 37),     // #181825
            tab_inactive_fg: Color::Rgb(255, 0, 127),    // #ff007f (Neon Magenta)
            leader_bg: Color::Rgb(255, 230, 0),         // #ffe600 (Acid Yellow)
            leader_fg: Color::Rgb(10, 10, 15),          // #0a0a0f
            status_busy: Color::Rgb(255, 230, 0),        // #ffe600
            status_waiting: Color::Rgb(0, 240, 255),     // #00f0ff
            status_done: Color::Rgb(0, 255, 102),        // #00ff66
            status_error: Color::Rgb(255, 0, 85),        // #ff0055
            border_focused: Color::Rgb(255, 0, 127),     // #ff007f
            border_unfocused: Color::Rgb(50, 50, 70),    // #323246
            selection_bg: Color::Rgb(255, 0, 127),       // #ff007f
            selection_fg: Color::Rgb(255, 255, 255),     // #ffffff
            selection_indicator: Color::Rgb(0, 240, 255), // #00f0ff
            modal_title: Color::Rgb(255, 0, 127),        // #ff007f
            modal_label: Color::Rgb(0, 240, 255),        // #00f0ff
            input_text: Color::Rgb(255, 230, 0),         // #ffe600
            input_placeholder: Color::Rgb(120, 120, 150), // #787896
            key_hint: Color::Rgb(0, 240, 255),          // #00f0ff
            error_banner_bg: Color::Rgb(255, 0, 85),     // #ff0055
            error_banner_fg: Color::Rgb(255, 255, 255),  // #ffffff
        }
    }

    /// Gruvbox / Gruvbox Dark: Warm retro earthy palette (`#282828`), rust orange active tabs (`#fe8019`), warm cream text (`#ebdbb2`), forest green selections (`#b8bb26`), and bright yellow leader mode (`#fabd2f`).
    pub fn gruvbox() -> Self {
        Self {
            name: "gruvbox".to_string(),
            status_bar_bg: Color::Rgb(40, 40, 40),       // #282828
            status_bar_fg: Color::Rgb(235, 219, 178),   // #ebdbb2
            tab_active_bg: Color::Rgb(254, 128, 25),     // #fe8019
            tab_active_fg: Color::Rgb(40, 40, 40),       // #282828
            tab_inactive_bg: Color::Rgb(60, 56, 54),     // #3c3836
            tab_inactive_fg: Color::Rgb(213, 196, 161), // #d5c4a1
            leader_bg: Color::Rgb(250, 189, 47),        // #fabd2f
            leader_fg: Color::Rgb(40, 40, 40),          // #282828
            status_busy: Color::Rgb(250, 189, 47),       // #fabd2f
            status_waiting: Color::Rgb(131, 165, 152),   // #83a598
            status_done: Color::Rgb(184, 187, 38),       // #b8bb26
            status_error: Color::Rgb(251, 73, 52),       // #fb4934
            border_focused: Color::Rgb(254, 128, 25),    // #fe8019
            border_unfocused: Color::Rgb(102, 92, 84),   // #665c54
            selection_bg: Color::Rgb(184, 187, 38),      // #b8bb26
            selection_fg: Color::Rgb(40, 40, 40),       // #282828
            selection_indicator: Color::Rgb(254, 128, 25), // #fe8019
            modal_title: Color::Rgb(250, 189, 47),       // #fabd2f
            modal_label: Color::Rgb(254, 128, 25),       // #fe8019
            input_text: Color::Rgb(250, 189, 47),        // #fabd2f
            input_placeholder: Color::Rgb(146, 131, 116), // #928374
            key_hint: Color::Rgb(131, 165, 152),        // #83a598
            error_banner_bg: Color::Rgb(251, 73, 52),    // #fb4934
            error_banner_fg: Color::Rgb(235, 219, 178),  // #ebdbb2
        }
    }

    /// Monochrome / Slate / Minimal / Zinc: Minimalist zinc/slate dark tones (`#18181b` / `#27272a`), ice blue accents, crisp white active text and borders (`#f4f4f5`).
    pub fn monochrome() -> Self {
        Self {
            name: "monochrome".to_string(),
            status_bar_bg: Color::Rgb(24, 24, 27),       // #18181b (Zinc 900)
            status_bar_fg: Color::Rgb(244, 244, 245),   // #f4f4f5 (Zinc 100)
            tab_active_bg: Color::Rgb(244, 244, 245),   // #f4f4f5 (Zinc 100)
            tab_active_fg: Color::Rgb(24, 24, 27),       // #18181b (Zinc 900)
            tab_inactive_bg: Color::Rgb(39, 39, 42),     // #27272a (Zinc 800)
            tab_inactive_fg: Color::Rgb(161, 161, 170), // #a1a1aa (Zinc 400)
            leader_bg: Color::Rgb(228, 228, 231),       // #e4e4e7 (Zinc 200)
            leader_fg: Color::Rgb(24, 24, 27),          // #18181b
            status_busy: Color::Rgb(228, 228, 231),      // #e4e4e7
            status_waiting: Color::Rgb(186, 230, 253),   // #bae6fd (Ice blue)
            status_done: Color::Rgb(244, 244, 245),      // #f4f4f5
            status_error: Color::Rgb(248, 113, 113),     // #f87171
            border_focused: Color::Rgb(244, 244, 245),   // #f4f4f5
            border_unfocused: Color::Rgb(82, 82, 91),    // #52525b (Zinc 600)
            selection_bg: Color::Rgb(244, 244, 245),     // #f4f4f5
            selection_fg: Color::Rgb(24, 24, 27),       // #18181b
            selection_indicator: Color::Rgb(186, 230, 253), // #bae6fd
            modal_title: Color::Rgb(244, 244, 245),      // #f4f4f5
            modal_label: Color::Rgb(228, 228, 231),      // #e4e4e7
            input_text: Color::Rgb(244, 244, 245),       // #f4f4f5
            input_placeholder: Color::Rgb(113, 113, 122), // #71717a (Zinc 500)
            key_hint: Color::Rgb(186, 230, 253),        // #bae6fd (Ice blue)
            error_banner_bg: Color::Rgb(248, 113, 113),  // #f87171
            error_banner_fg: Color::Rgb(24, 24, 27),     // #18181b
        }
    }

    /// ANSI: Standard 16-color ANSI terminal fallback palette using native terminal cyan, yellow, green, red, dark gray, and white without 24-bit TrueColor RGB definitions.
    pub fn ansi() -> Self {
        Self {
            name: "ansi".to_string(),
            status_bar_bg: Color::DarkGray,
            status_bar_fg: Color::White,
            tab_active_bg: Color::Cyan,
            tab_active_fg: Color::Black,
            tab_inactive_bg: Color::DarkGray,
            tab_inactive_fg: Color::White,
            leader_bg: Color::Yellow,
            leader_fg: Color::Black,
            status_busy: Color::Yellow,
            status_waiting: Color::Cyan,
            status_done: Color::Green,
            status_error: Color::Red,
            border_focused: Color::Cyan,
            border_unfocused: Color::DarkGray,
            selection_bg: Color::Cyan,
            selection_fg: Color::Black,
            selection_indicator: Color::Cyan,
            modal_title: Color::Yellow,
            modal_label: Color::Cyan,
            input_text: Color::Yellow,
            input_placeholder: Color::Gray,
            key_hint: Color::LightCyan,
            error_banner_bg: Color::Red,
            error_banner_fg: Color::White,
        }
    }

    /// Resolves a `Theme` from a given theme name or alias.
    ///
    /// Matches case-insensitively and normalizes underscores/hyphens.
    /// Falls back to `tokyo_night()` if unrecognized.
    pub fn from_name(name: &str) -> Self {
        let normalized = name.trim().to_lowercase().replace('_', "-");
        match normalized.as_str() {
            "tokyo-night" | "tokyonight" | "tokyo" => Self::tokyo_night(),
            "catppuccin" | "catppuccin-mocha" | "mocha" => Self::catppuccin(),
            "cyberpunk" | "synthwave" => Self::cyberpunk(),
            "gruvbox" | "gruvbox-dark" => Self::gruvbox(),
            "monochrome" | "slate" | "minimal" | "zinc" => Self::monochrome(),
            "ansi" | "default" | "classic" | "standard" => Self::ansi(),
            _ => Self::tokyo_night(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::tokyo_night()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default_is_tokyo_night() {
        let default_theme = Theme::default();
        let tokyo_night = Theme::tokyo_night();
        assert_eq!(default_theme.name, "tokyo-night");
        assert_eq!(default_theme, tokyo_night);
    }

    #[test]
    fn test_theme_from_name_exact() {
        assert_eq!(Theme::from_name("tokyo-night"), Theme::tokyo_night());
        assert_eq!(Theme::from_name("catppuccin"), Theme::catppuccin());
        assert_eq!(Theme::from_name("cyberpunk"), Theme::cyberpunk());
        assert_eq!(Theme::from_name("gruvbox"), Theme::gruvbox());
        assert_eq!(Theme::from_name("monochrome"), Theme::monochrome());
        assert_eq!(Theme::from_name("ansi"), Theme::ansi());
    }

    #[test]
    fn test_theme_from_name_aliases_and_normalization() {
        // Catppuccin aliases
        assert_eq!(Theme::from_name("Catppuccin-Mocha"), Theme::catppuccin());
        assert_eq!(Theme::from_name("mocha"), Theme::catppuccin());
        assert_eq!(Theme::from_name("catppuccin_mocha"), Theme::catppuccin());

        // Cyberpunk aliases
        assert_eq!(Theme::from_name("SYNTHWAVE"), Theme::cyberpunk());
        assert_eq!(Theme::from_name("Cyberpunk"), Theme::cyberpunk());

        // Gruvbox aliases
        assert_eq!(Theme::from_name("gruvbox-dark"), Theme::gruvbox());
        assert_eq!(Theme::from_name("Gruvbox_Dark"), Theme::gruvbox());

        // Monochrome aliases
        assert_eq!(Theme::from_name("slate"), Theme::monochrome());
        assert_eq!(Theme::from_name("MINIMAL"), Theme::monochrome());
        assert_eq!(Theme::from_name("zinc"), Theme::monochrome());

        // ANSI aliases
        assert_eq!(Theme::from_name("default"), Theme::ansi());
        assert_eq!(Theme::from_name("classic"), Theme::ansi());
        assert_eq!(Theme::from_name("standard"), Theme::ansi());
        assert_eq!(Theme::from_name("AnSi"), Theme::ansi());

        // Tokyo Night aliases
        assert_eq!(Theme::from_name("tokyonight"), Theme::tokyo_night());
        assert_eq!(Theme::from_name("tokyo"), Theme::tokyo_night());
        assert_eq!(Theme::from_name("TOKYO_NIGHT"), Theme::tokyo_night());
    }

    #[test]
    fn test_theme_from_name_fallback_unknown() {
        assert_eq!(Theme::from_name("unknown-palette"), Theme::tokyo_night());
        assert_eq!(Theme::from_name(""), Theme::tokyo_night());
        assert_eq!(Theme::from_name("   "), Theme::tokyo_night());
        assert_eq!(Theme::from_name("dracula"), Theme::tokyo_night());
    }

    #[test]
    fn test_theme_palettes_have_distinct_names() {
        let palettes = [
            Theme::tokyo_night(),
            Theme::catppuccin(),
            Theme::cyberpunk(),
            Theme::gruvbox(),
            Theme::monochrome(),
            Theme::ansi(),
        ];

        for i in 0..palettes.len() {
            for j in (i + 1)..palettes.len() {
                assert_ne!(
                    palettes[i], palettes[j],
                    "Palette {} should not equal palette {}",
                    palettes[i].name, palettes[j].name
                );
            }
        }
    }
}

use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub(super) struct AppLayout {
    pub(super) header: Rect,
    pub(super) search: Rect,
    pub(super) filters: Rect,
    pub(super) main: MainLayout,
    pub(super) footer: Rect,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MainLayout {
    ResultsOnly { results: Rect },
    Split { results: Rect, preview: Rect },
}

impl MainLayout {
    pub(super) fn results(self) -> Rect {
        match self {
            Self::ResultsOnly { results } | Self::Split { results, .. } => results,
        }
    }

    pub(super) fn preview(self) -> Option<Rect> {
        match self {
            Self::ResultsOnly { .. } => None,
            Self::Split { preview, .. } => Some(preview),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScrollTarget {
    Results,
    Preview,
}

pub(super) fn app(area: Rect, show_preview: bool, preview_ratio: u16) -> AppLayout {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    AppLayout {
        header: outer[0],
        search: outer[1],
        filters: outer[2],
        main: main(outer[3], show_preview, preview_ratio),
        footer: outer[4],
    }
}

pub(super) fn scroll_target(
    area: Rect,
    show_preview: bool,
    preview_ratio: u16,
    column: u16,
    row: u16,
) -> Option<ScrollTarget> {
    let main = app(area, show_preview, preview_ratio).main;
    if contains(main.results(), column, row) {
        return Some(ScrollTarget::Results);
    }
    if main
        .preview()
        .is_some_and(|preview| contains(preview, column, row))
    {
        return Some(ScrollTarget::Preview);
    }
    None
}

fn main(area: Rect, show_preview: bool, preview_ratio: u16) -> MainLayout {
    if !show_preview {
        return MainLayout::ResultsOnly { results: area };
    }

    if area.width >= 116 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(100 - preview_ratio),
                Constraint::Percentage(preview_ratio),
            ])
            .split(area);
        MainLayout::Split {
            results: chunks[0],
            preview: chunks[1],
        }
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(12)])
            .split(area);
        MainLayout::Split {
            results: chunks[0],
            preview: chunks[1],
        }
    }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATIO: u16 = 38;

    #[test]
    fn hit_tests_horizontal_results_and_preview() {
        let area = Rect::new(0, 0, 120, 40);

        assert_eq!(
            scroll_target(area, true, RATIO, 10, 6),
            Some(ScrollTarget::Results)
        );
        assert_eq!(
            scroll_target(area, true, RATIO, 100, 6),
            Some(ScrollTarget::Preview)
        );
    }

    #[test]
    fn hit_tests_vertical_results_and_preview() {
        let area = Rect::new(0, 0, 80, 40);

        assert_eq!(
            scroll_target(area, true, RATIO, 10, 8),
            Some(ScrollTarget::Results)
        );
        assert_eq!(
            scroll_target(area, true, RATIO, 10, 30),
            Some(ScrollTarget::Preview)
        );
    }

    #[test]
    fn preview_hidden_routes_main_area_to_results() {
        let area = Rect::new(0, 0, 120, 40);

        assert_eq!(
            scroll_target(area, false, RATIO, 100, 6),
            Some(ScrollTarget::Results)
        );
        assert_eq!(scroll_target(area, false, RATIO, 100, 1), None);
    }

    #[test]
    fn horizontal_split_width_follows_preview_ratio() {
        let area = Rect::new(0, 0, 120, 40);
        let MainLayout::Split { results, preview } = main(area, true, 50) else {
            panic!("expected a split layout");
        };

        assert_eq!(results.width + preview.width, area.width);
        // A 50% ratio should give the preview roughly half of the 120-wide area.
        assert!(
            preview.width.abs_diff(60) <= 1,
            "preview width {} not near 50% of 120",
            preview.width
        );
    }
}

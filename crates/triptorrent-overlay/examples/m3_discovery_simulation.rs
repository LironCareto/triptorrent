use triptorrent_overlay::research::{render_markdown, run_default_research_suite};

fn main() {
    print!("{}", render_markdown(&run_default_research_suite()));
}

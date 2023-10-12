fn main() {
    let git_commit = build_data::get_git_commit_short().unwrap_or_else(|_| "unknown".to_string());
    let git_dirty = build_data::get_git_dirty().unwrap_or_default();
    let build_date = build_data::format_date(build_data::now());

    let build_info = if !git_commit.is_empty() {
        let git_info = if git_dirty {
            format!("{git_commit}-dirty")
        } else {
            git_commit
        };

        format!("({git_info} {build_date})")
    } else {
        build_date.to_string()
    };

    println!("cargo:rustc-env=CARGO_BUILD_INFO={build_info}");
    build_data::no_debug_rebuilds();
}

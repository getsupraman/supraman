use super::scanner_held;

#[test]
fn only_the_refusals_a_scanner_causes_are_tried_again() {
    for held in [
        "fatal: cannot change to 'P:/system/temp/.tmpAb12/repo': Invalid argument\n",
        "fatal: cannot copy 'C:/Program Files/Git/mingw64/share/git-core/templates/hooks/pre-push.sample' to 'P:/system/temp/.tmpAb12/mine/.git/hooks/pre-push.sample': Permission denied\n",
        "fatal: this operation must be run in a work tree\n",
    ] {
        assert!(scanner_held(held), "{held}");
    }
    for refused in [
        "",
        "fatal: cannot change to 'P:/gone': No such file or directory\n",
        "error: open(\"a.txt\"): Permission denied\n",
        "fatal: not a git repository (or any of the parent directories): .git\n",
        "error: pathspec 'x' did not match any file(s) known to git\n",
    ] {
        assert!(!scanner_held(refused), "{refused}");
    }
}

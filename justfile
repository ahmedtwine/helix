profile := env("HELIX_PROFILE", "release")
bin_dir := env("HELIX_BIN_DIR", home_directory() / ".local" / "bin")
runtime_dir := home_directory() / ".config" / "helix" / "runtime"
grammars_built := env("HELIX_SKIP_GRAMMARS", path_exists(justfile_directory() / "runtime" / "grammars" / "rust.dylib"))
skip_grammars := if grammars_built == "true" { "HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1" } else { "" }

export RUSTC_WRAPPER := `command -v sccache || true`
export CARGO_INCREMENTAL := "0"

sync:
    @git diff-index --quiet HEAD -- || { echo "working tree is dirty, commit or stash first"; exit 1; }
    @git remote get-url upstream >/dev/null 2>&1 || git remote add upstream https://github.com/helix-editor/helix.git
    git fetch upstream --tags
    git checkout master
    git merge --ff-only upstream/master
    git push origin master
    git checkout studio
    git merge master --no-edit
    git log --oneline master..studio

install:
    {{ skip_grammars }} cargo build --profile {{ profile }} --locked
    mkdir -p {{ bin_dir }} {{ parent_directory(runtime_dir) }}
    install -m 755 {{ justfile_directory() }}/target/{{ profile }}/hx {{ bin_dir }}/hx
    ln -sfn {{ justfile_directory() }}/runtime {{ runtime_dir }}
    {{ bin_dir }}/hx --version

# Runs cargo with VS 2022's bundled CMake on PATH for this process only.
# wasmtime-c-api-impl 48.0.7 (settings -> migrator -> tree-sitter[wasm]) spawns `cmake` from PATH.
# Usage: pwsh -File scripts/cargo.ps1 build --release -p term-spike
$env:PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$env:PATH"
& cargo @args
exit $LASTEXITCODE

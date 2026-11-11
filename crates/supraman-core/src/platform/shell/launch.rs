//! The PowerShell text a [+] tab runs claude with (C113: tabs that start claude stay on PowerShell).

/// `s` as a single-quoted PowerShell string.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Under legacy arg passing (5.1, pwsh < 7.3) claude gets one MSVC-quoted command line, never via its environment.
const RUNNER: &str = r#"function supramanClaude($supramanArgv) {
    $c = Get-Command -Name <command> -ErrorAction SilentlyContinue | Select-Object -First 1
    if ("$PSNativeCommandArgumentPassing" -in '', 'Legacy' -and $c.CommandType -eq 'Application' -and $c.Extension -notin '.cmd', '.bat') {
        $line = @(@(& { $args }<args>) + $supramanArgv | ForEach-Object { if ($_ -ne '' -and $_ -notmatch '[\s"]') { $_ } else { '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } }) -join ' '
        $p = [Diagnostics.Process]::Start((New-Object Diagnostics.ProcessStartInfo -Property @{ FileName = $c.Path; Arguments = $line; UseShellExecute = $false; WorkingDirectory = (Get-Location -PSProvider FileSystem).ProviderPath }))
        $p.WaitForExit(); $global:LASTEXITCODE = $p.ExitCode
    } else { <claude> @supramanArgv }
}
"#;

/// The `supramanClaude` function for `claude`: a bare name, or `& '<path>' …` with its args.
pub fn runner(claude: &str) -> String {
    let (word, args) = command_split(claude);
    RUNNER.replace("<command>", word).replace("<args>", args).replace("<claude>", claude)
}

/// The path of `& '<path>' …`, still quoted, else the first word; then the args after it, as PowerShell source.
fn command_split(claude: &str) -> (&str, &str) {
    let claude = claude.trim_start();
    if let Some(rest) = claude.strip_prefix("& '") {
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                } else {
                    return (&claude[2..3 + i + 1], &claude[3 + i + 1..]);
                }
            }
        }
    }
    let end = claude.find(char::is_whitespace).unwrap_or(claude.len());
    (&claude[..end], &claude[end..])
}

/// `script` as `-EncodedCommand` takes it: UTF-16LE in base64.
pub fn encoded_command(script: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_resolves_the_command_word_of_a_bare_or_quoted_claude() {
        assert_eq!(command_split("claude"), ("claude", ""));
        assert_eq!(command_split("claude --resume 'x'"), ("claude", " --resume 'x'"));
        assert_eq!(command_split(r"& 'C:\o''neil\claude.exe' --selftest-argv 'out'"), (r"'C:\o''neil\claude.exe'", " --selftest-argv 'out'"));
        assert!(runner(r"& 'C:\x\c.exe' -a").contains(r"Get-Command -Name 'C:\x\c.exe' "));
    }
}

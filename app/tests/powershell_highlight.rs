//! The bundled PowerShell grammar (vendor/gpui-component-0.8.0) is picked up for the script library and the editor.

use gpui_kit::component::highlighter::debug_highlight_names;

#[test]
fn powershell_scripts_are_highlighted() {
    for token in ["powershell", "ps1", "PowerShell"] {
        assert!(!debug_highlight_names(token, "Get-Date").is_empty(), "`{token}` has no grammar");
    }
    let code = "function Get-Thing($Name) {\n  # note\n  Get-ChildItem -Path $env:TEMP | Where-Object { $_.Length -gt 10 -and $Name -eq 'x' }\n  \"hi $Name\" ; [int]$n = 0x1F ; $true\n}\n";
    let got = debug_highlight_names("powershell", code);
    let has = |name: &str, text: &str| got.iter().any(|(n, t)| *n == name && t.contains(text));
    assert!(has("keyword", "function"), "{got:?}");
    assert!(has("function", "Get-Thing"), "{got:?}");
    assert!(has("comment", "# note"), "{got:?}");
    assert!(has("function", "Get-ChildItem") && has("function", "Where-Object"), "{got:?}");
    assert!(has("variable", "$env:TEMP"), "{got:?}");
    assert!(has("operator", "-gt") && has("operator", "-eq"), "{got:?}");
    assert!(has("string", "'x'") && has("string", "hi"), "{got:?}");
    assert!(has("type", "[int]") && has("number", "0x1F"), "{got:?}");
    assert!(has("boolean", "$true"), "{got:?}");
    // block comments and here-strings span lines
    let multi = "<# one\ntwo #>\n@\"\nline $x\n\"@\nWrite-Host ok\n";
    let got = debug_highlight_names("powershell", multi);
    assert!(got.iter().any(|(n, t)| *n == "comment" && t.contains("two")), "{got:?}");
    assert!(got.iter().any(|(n, t)| *n == "string" && t.contains("line")), "{got:?}");
    assert!(got.iter().any(|(n, t)| *n == "function" && t.contains("Write-Host")), "{got:?}");
    // other languages keep working
    assert!(!debug_highlight_names("bash", "echo hi # c").is_empty());
}

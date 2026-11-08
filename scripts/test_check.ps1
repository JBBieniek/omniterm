# Runner contract tests use fake tools in a test-owned copy, never as Rust evidence.
$ErrorActionPreference = 'Stop'
$shell = (Get-Process -Id $PID).Path
$runner = Join-Path $PSScriptRoot 'check.ps1'
$fixture = Join-Path $PSScriptRoot ("../artifacts/local/runner-contract-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path "$fixture/scripts", "$fixture/tools" -Force | Out-Null
New-Item -ItemType Directory -Path "$fixture/apps/desktop/node_modules", "$fixture/apps/desktop/src-tauri/gen" -Force | Out-Null
New-Item -ItemType Directory -Path "$fixture/tools/desktop-test-host/src" -Force | Out-Null
Copy-Item -LiteralPath $runner -Destination "$fixture/scripts/check.ps1"
Set-Content -LiteralPath "$fixture/untracked.rs" -Value '// source identity fixture'
Set-Content -LiteralPath "$fixture/apps/desktop/App.svelte" -Value '<h1>fixture UI</h1>'
Set-Content -LiteralPath "$fixture/tools/desktop-test-host/src/automation.rs" -Value '// test host source fixture'
Set-Content -LiteralPath "$fixture/apps/desktop/node_modules/generated.js" -Value 'excluded dependency'
Set-Content -LiteralPath "$fixture/apps/desktop/src-tauri/gen/generated.json" -Value '{}'
Set-Content -LiteralPath "$fixture/tools/cargo.cmd" -Value @'
@echo off
echo fixture cargo %*
if "%1"=="test" if "%OMNITERM_FIXTURE_FAIL%"=="1" exit /b 23
if "%1"=="test" if "%OMNITERM_FIXTURE_DRIFT%"=="1" echo // drift>>"%~dp0..\untracked.rs"
exit /b 0
'@
Set-Content -LiteralPath "$fixture/tools/rustc.cmd" -Value '@echo fixture rustc'

function Assert($condition, $message) {
    if (-not $condition) { throw $message }
}

$savedPath = $env:PATH
$savedFail = $env:OMNITERM_FIXTURE_FAIL
$savedDrift = $env:OMNITERM_FIXTURE_DRIFT
try {
    # No Git or real Cargo is visible in the child; identity must still be useful.
    $env:PATH = "$fixture/tools"
    $env:OMNITERM_FIXTURE_FAIL = '0'
    $env:OMNITERM_FIXTURE_DRIFT = '0'
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/pass"
    Assert ($LASTEXITCODE -eq 0) 'Successful checks must exit zero.'
    $passed = Get-Content "$fixture/pass/run.json" -Raw | ConvertFrom-Json
    Assert ($passed.result -eq 'PASS') 'Successful run must report PASS.'
    Assert ($passed.checks.Count -eq 4) 'All four real check stages must be represented.'
    Assert ($passed.source.sha256 -match '^[a-f0-9]{64}$') 'Source hash is required without Git.'
    Assert ((Get-Content "$fixture/pass/source.sha256" -Raw) -match 'untracked.rs') 'Untracked source must be fingerprinted.'
    $manifest = Get-Content "$fixture/pass/source.sha256" -Raw
    Assert ($manifest -match 'apps/desktop/App.svelte') 'Desktop source must be fingerprinted.'
    Assert ($manifest -match 'tools/desktop-test-host/src/automation.rs') 'Separate test host source must be fingerprinted.'
    Assert ($passed.source.scope -match 'apps/tools/docs') 'Recorded source scope must include the test host root.'
    Assert ($manifest -notmatch 'generated') 'Generated and dependency trees must be excluded.'
    Assert ($passed.checks[1].command -match '-p omniterm-core') 'Core checks must remain independent of native packages.'
    Assert ($passed.git.result -eq 'UNAVAILABLE') 'Unavailable Git must be explicit.'

    $env:OMNITERM_FIXTURE_FAIL = '1'
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/fail"
    Assert ($LASTEXITCODE -ne 0) 'Failed Cargo must exit nonzero.'
    $failed = Get-Content "$fixture/fail/run.json" -Raw | ConvertFrom-Json
    Assert ($failed.result -eq 'FAIL') 'Failed Cargo must report FAIL.'
    Assert ($failed.checks[1].exit_code -eq 23) 'Original failure code must be retained.'
    Assert ($failed.checks[2].result -eq 'NOT RUN') 'Checks after failure must stay NOT RUN.'
    Assert ((Get-Content "$fixture/fail/test.txt" -Raw) -match 'fixture cargo test') 'Failed command output must be retained.'

    $env:PATH = "$fixture/empty"
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/missing"
    Assert ($LASTEXITCODE -ne 0) 'Missing Cargo must exit nonzero.'
    $missing = Get-Content "$fixture/missing/run.json" -Raw | ConvertFrom-Json
    Assert ($missing.result -eq 'BLOCKED') 'Missing tools must report BLOCKED.'
    Assert ($missing.checks[0].result -eq 'NOT RUN') 'Missing tools cannot pass checks.'

    $env:PATH = "$fixture/tools"
    $env:OMNITERM_FIXTURE_FAIL = '0'
    $before = Get-Content "$fixture/pass/run.json" -Raw
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/pass"
    Assert ($LASTEXITCODE -ne 0) 'An existing run directory must be rejected.'
    Assert ((Get-Content "$fixture/pass/run.json" -Raw) -eq $before) 'Existing evidence must survive.'
    Add-Content -LiteralPath "$fixture/untracked.rs" -Value '// changed'
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/changed"
    Assert ($LASTEXITCODE -eq 0) 'Changed source fixture must complete.'
    $changed = Get-Content "$fixture/changed/run.json" -Raw | ConvertFrom-Json
    Assert ($changed.source.sha256 -ne $passed.source.sha256) 'Changed untracked source must change identity.'
    $env:OMNITERM_FIXTURE_DRIFT = '1'
    & $shell -NoProfile -File "$fixture/scripts/check.ps1" -OutputDirectory "$fixture/drift"
    Assert ($LASTEXITCODE -ne 0) 'Source edits during checks must invalidate a pass.'
    $drift = Get-Content "$fixture/drift/run.json" -Raw | ConvertFrom-Json
    Assert ($drift.result -eq 'FAIL' -and $drift.error -match 'Source changed') 'Source drift must explain the failed run.'
    Write-Host "Runner contracts PASS (six scenarios, fake tools only). Evidence: $fixture"
} finally {
    $env:PATH = $savedPath
    $env:OMNITERM_FIXTURE_FAIL = $savedFail
    $env:OMNITERM_FIXTURE_DRIFT = $savedDrift
}

param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Shard', 'Collect')]
    [string]$Mode,
    [string]$TargetDir = $env:KURU_COVERAGE_TARGET,
    [string]$ExpectedSource = $env:KURU_COVERAGE_SOURCE,
    [string]$RunAttempt = $env:KURU_COVERAGE_ATTEMPT,
    [string]$Shard = $env:KURU_COVERAGE_SHARD,
    [string]$Packages = $env:KURU_COVERAGE_PACKAGES,
    [string]$OutputDir = $env:KURU_COVERAGE_OUTPUT,
    [string]$Inputs = $env:KURU_COVERAGE_INPUTS,
    [string]$OutputPath = $env:KURU_COVERAGE_REPORT,
    [string]$Diagnostics = $env:KURU_COVERAGE_DIAGNOSTICS,
    [string]$JobStarted = $env:KURU_COVERAGE_JOB_STARTED,
    [string]$JobMinutes = $env:KURU_COVERAGE_JOB_MINUTES
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if ([string]::IsNullOrWhiteSpace($TargetDir) -or
    [string]::IsNullOrWhiteSpace($ExpectedSource) -or
    [string]::IsNullOrWhiteSpace($RunAttempt)) {
    throw 'Coverage target, expected source and run attempt are required'
}
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Set-Location $root

if (Test-Path -LiteralPath $TargetDir) {
    throw "Coverage target already exists: $TargetDir"
}
[void](New-Item -ItemType Directory -Path $TargetDir)
$target = (Resolve-Path -LiteralPath $TargetDir).Path
$state = Join-Path $target 'kuru-shard-state'
[void](New-Item -ItemType Directory -Path $state)

# Bundle preparation has already built this package-owned helper outside the
# instrumented target. Do not invoke Cargo for the helper after show-env.
& cargo build -p kuru-delivery --features tooling --locked --bin kuru-delivery
if ($LASTEXITCODE -ne 0) { throw 'Building the coverage verifier failed' }
$helper = (Resolve-Path 'target/debug/kuru-delivery.exe').Path
$llvmCovRoot = (& mise bin-paths 'cargo:cargo-llvm-cov@0.9.1').Trim()
$llvmCov = Join-Path $llvmCovRoot 'cargo-llvm-cov.exe'
if (-not (Test-Path -LiteralPath $llvmCov -PathType Leaf)) {
    throw "Pinned cargo-llvm-cov executable is missing: $llvmCov"
}
$version = (& $llvmCov llvm-cov --version).Trim()
if ($version -ne 'cargo-llvm-cov 0.9.1') {
    throw "Expected cargo-llvm-cov 0.9.1, got $version"
}
& $helper coverage verify-source --root $root --expected-source $ExpectedSource --llvm-cov $llvmCov
if ($LASTEXITCODE -ne 0) { throw 'Coverage source verification failed' }

$env:CARGO_TARGET_DIR = $target
$env:CARGO_LLVM_COV_TARGET_DIR = $target
$env:LLVM_PROFILE_FILE = Join-Path $target 'kuru-%p-%m.profraw'
$coverageEnvironment = (& $llvmCov llvm-cov show-env --pwsh) -join [Environment]::NewLine
if ($LASTEXITCODE -ne 0) { throw 'cargo-llvm-cov show-env failed' }
Invoke-Expression $coverageEnvironment

function Invoke-CargoJson {
    param([string[]]$Arguments, [string]$Path)
    & cargo @Arguments | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
    if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments -join ' ') failed" }
}

$metadata = Join-Path $state 'metadata.json'
Invoke-CargoJson -Arguments @('metadata', '--format-version=1', '--no-deps', '--locked') -Path $metadata
$fullMessages = Join-Path $state 'full-messages.json'
$fullArgs = @(
    'test', '--workspace', '--all-targets', '--all-features', '--locked',
    '--no-run', '--message-format=json-render-diagnostics'
)
Invoke-CargoJson -Arguments $fullArgs -Path $fullMessages
$inventory = Join-Path $state 'inventory.json'
& $helper coverage inventory --metadata $metadata --messages $fullMessages --target-dir $target --output $inventory
if ($LASTEXITCODE -ne 0) { throw 'Full coverage inventory validation failed' }

if ($Mode -eq 'Shard') {
    if ([string]::IsNullOrWhiteSpace($Shard) -or [string]::IsNullOrWhiteSpace($Packages) -or [string]::IsNullOrWhiteSpace($OutputDir)) {
        throw 'Shard mode requires -Shard, -Packages and -OutputDir'
    }
    if ([string]::IsNullOrWhiteSpace($Diagnostics) -or
        [string]::IsNullOrWhiteSpace($JobStarted) -or
        [string]::IsNullOrWhiteSpace($JobMinutes)) {
        throw 'Shard mode requires -Diagnostics, -JobStarted and -JobMinutes'
    }
    if (Test-Path -LiteralPath $Diagnostics) {
        throw "Coverage diagnostics already exist: $Diagnostics"
    }
    [void](New-Item -ItemType Directory -Path $Diagnostics)
    $diagnosticsDir = (Resolve-Path -LiteralPath $Diagnostics).Path
    $selectedPackages = @($Packages.Split(',', [System.StringSplitOptions]::RemoveEmptyEntries) | Sort-Object -Unique)
    $workspacePackages = @(
        'kuru', 'kuru-archive', 'kuru-connectors', 'kuru-core',
        'kuru-delivery', 'kuru-memory', 'kuru-platform', 'kuru-runtime'
    )
    foreach ($package in $selectedPackages) {
        if ($package -notin $workspacePackages) { throw "Unknown coverage package: $package" }
    }
    $selection = Join-Path $state 'selection.json'
    & $helper coverage selection --inventory $inventory --packages ($selectedPackages -join ',') --output $selection
    if ($LASTEXITCODE -ne 0) { throw 'Selected coverage inventory validation failed' }

    $rustcIdentity = & rustc -vV
    if ($LASTEXITCODE -ne 0) { throw 'Reading the Rust host target failed' }
    $hostTarget = @($rustcIdentity | Where-Object { $_ -like 'host: *' })
    if ($hostTarget.Count -ne 1) { throw 'Rust identity did not contain one host target' }
    $hostTarget = $hostTarget[0].Substring('host: '.Length)
    $ledger = Join-Path $state 'runner-ledger.jsonl'
    $runnerConfig = Join-Path $state 'runner-config.toml'
    # The runner's test deadline sits inside the hosted job limit, so a stalled
    # test fails here with evidence instead of being cancelled by the host.
    & $helper coverage runner-config --root $root --host $hostTarget --helper $helper --inventory $inventory --selection $selection --target-dir $target --ledger $ledger --diagnostics $diagnosticsDir --job-started $JobStarted --job-minutes $JobMinutes --output $runnerConfig
    if ($LASTEXITCODE -ne 0) { throw 'Cargo runner configuration failed' }

    try {
        # Compile-phase profiles are not test evidence. The runner below executes
        # the same full Cargo graph while omitting only unassigned test binaries.
        & $helper coverage discard-compile-profiles --profiles $target
        if ($LASTEXITCODE -ne 0) { throw 'Compile-profile isolation failed' }
        $runArgs = @(
            '--config', $runnerConfig, 'test', '--workspace', '--all-targets',
            '--all-features', '--locked', '--no-fail-fast'
        )
        & cargo @runArgs
        if ($LASTEXITCODE -ne 0) { throw 'Coverage shard tests failed' }
        & $helper coverage validate-run --inventory $inventory --selection $selection --ledger $ledger
        if ($LASTEXITCODE -ne 0) { throw 'Cargo runner ledger validation failed' }
        & $helper coverage receipt --root $root --inventory $inventory --selection $selection --ledger $ledger --profiles $target --shard $Shard --run-attempt $RunAttempt --expected-source $ExpectedSource --llvm-cov $llvmCov --output $OutputDir
        if ($LASTEXITCODE -ne 0) { throw 'Coverage shard receipt failed' }
    } catch {
        # Keep the shard's manifests and runner ledger beside the per-test
        # output logs and stall reports for the failure diagnostics upload.
        foreach ($name in @('inventory.json', 'selection.json', 'runner-config.toml', 'runner-ledger.jsonl')) {
            $source = Join-Path $state $name
            if (Test-Path -LiteralPath $source -PathType Leaf) {
                Copy-Item -LiteralPath $source -Destination (Join-Path $diagnosticsDir $name)
            }
        }
        throw
    }
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Inputs) -or [string]::IsNullOrWhiteSpace($OutputPath)) {
    throw 'Collect mode requires -Inputs and -OutputPath'
}
& $helper coverage discard-compile-profiles --profiles $target
if ($LASTEXITCODE -ne 0) { throw 'Compile-profile isolation failed' }
# Each shard contributes its latest uploaded attempt no later than this one.
& $helper coverage collect --root $root --inventory $inventory --inputs $Inputs --target-dir $target --expected-source $ExpectedSource --max-attempt $RunAttempt --llvm-cov $llvmCov
if ($LASTEXITCODE -ne 0) { throw 'Coverage shard aggregation failed' }
if (Test-Path -LiteralPath $OutputPath) {
    throw "Coverage report already exists: $OutputPath"
}
& $llvmCov llvm-cov report --failure-mode any --fail-under-lines 90 --lcov --output-path $OutputPath
if ($LASTEXITCODE -ne 0) { throw 'Workspace coverage report failed' }

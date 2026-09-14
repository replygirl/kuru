[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$required = @(
    'RELEASE_VERSION',
    'RELEASE_SHA',
    'RELEASE_RUN_URL',
    'KURU_PUBLISHED_WINDOWS_RECEIPT'
)
foreach ($name in $required) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw 'Published Windows verification requires RELEASE_VERSION, RELEASE_SHA, RELEASE_RUN_URL, and KURU_PUBLISHED_WINDOWS_RECEIPT.'
    }
}

$misePath = (Get-Command mise -CommandType Application).Source
& cargo run -p kuru-delivery --features tooling --locked --bin kuru-delivery -- verify-published-windows --mise $misePath --version $env:RELEASE_VERSION --expected-sha $env:RELEASE_SHA --run-url $env:RELEASE_RUN_URL --evidence $env:KURU_PUBLISHED_WINDOWS_RECEIPT
exit $LASTEXITCODE

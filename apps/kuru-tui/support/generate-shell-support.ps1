$ErrorActionPreference = 'Stop'
$binary = $env:KURU_SHELL_SUPPORT_BINARY
$output = $env:KURU_SHELL_SUPPORT_OUTPUT
if ([string]::IsNullOrWhiteSpace($binary) -or -not [IO.Path]::IsPathRooted($binary) -or
    -not [IO.File]::Exists($binary) -or [string]::IsNullOrWhiteSpace($output) -or
    -not [IO.Path]::IsPathRooted($output) -or [IO.Directory]::Exists($output) -or
    [IO.File]::Exists($output) -or -not [IO.Directory]::Exists([IO.Path]::GetDirectoryName($output))) {
    throw 'Select an existing absolute release binary and a new private output directory.'
}

[IO.Directory]::CreateDirectory($output) | Out-Null
[IO.Directory]::CreateDirectory([IO.Path]::Combine($output, 'completions')) | Out-Null
[IO.Directory]::CreateDirectory([IO.Path]::Combine($output, 'man')) | Out-Null

function Write-Generated([string]$arguments, [string]$destination) {
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $binary
    $start.Arguments = $arguments
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($start)
    if ($null -eq $process) { throw 'Shell-support generator did not start.' }
    try {
        $file = [IO.File]::Create($destination)
        try {
            $copy = $process.StandardOutput.BaseStream.CopyToAsync($file)
            $errorRead = $process.StandardError.ReadToEndAsync()
            if (-not $process.WaitForExit(60000)) {
                $process.Kill()
                throw 'Shell-support generator exceeded its bounded deadline.'
            }
            $copy.GetAwaiter().GetResult()
            $null = $errorRead.GetAwaiter().GetResult()
            $file.Flush($true)
        } finally {
            $file.Dispose()
        }
        if ($process.ExitCode -ne 0) { throw 'Shell-support generator failed.' }
        $length = ([IO.FileInfo]$destination).Length
        if ($length -lt 1 -or $length -gt 524288) { throw 'Generated shell-support file has invalid size.' }
    } finally {
        $process.Dispose()
    }
}

Write-Generated 'completions bash' ([IO.Path]::Combine($output, 'completions', 'kuru.bash'))
Write-Generated 'completions zsh' ([IO.Path]::Combine($output, 'completions', '_kuru'))
Write-Generated 'completions fish' ([IO.Path]::Combine($output, 'completions', 'kuru.fish'))
Write-Generated 'completions powershell' ([IO.Path]::Combine($output, 'completions', 'kuru.ps1'))
Write-Generated 'man' ([IO.Path]::Combine($output, 'man', 'kuru.1'))

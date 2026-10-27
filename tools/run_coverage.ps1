# Par C: decoding coverage and record digests over a sample set, one file at a time.
# Usage: pwsh tools/run_coverage.ps1 <rvt.exe> <graph_digest.exe> <out dir> <file.rvt>...
# Writes <out>/cov/<name>.json (rvt coverage) and <out>/dig/<name>.tsv (graph_digest with the
# file's extensible-storage schemas, RVT_DIGEST_ES=1).
param([string]$Cli, [string]$Digest, [string]$Out, [Parameter(ValueFromRemainingArguments = $true)][string[]]$Files)
New-Item -ItemType Directory -Force "$Out\cov", "$Out\dig" | Out-Null
$env:RVT_DIGEST_ES = '1'
foreach ($f in $Files) {
    $b = [IO.Path]::GetFileNameWithoutExtension($f)
    & $Cli coverage $f -o "$Out\cov\$b.json" 2> "$Out\cov\$b.err"
    if ($LASTEXITCODE -ne 0) { "FAIL coverage $b" }
    if ($Digest) {
        & $Digest $f "$Out\dig\$b.tsv" 2> "$Out\dig\$b.err"
        if ($LASTEXITCODE -ne 0) { "FAIL digest $b" }
    }
}

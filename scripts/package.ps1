param(
    [string] $ModelSource = 'https://huggingface.co',
    [ValidatePattern('^[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*$')]
    [string] $BundleName = 'TabSortLite-Edge-1.0.0',
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    $cache = Join-Path $root '.tools'
    $models = Join-Path $root 'models'
    $fullE5 = Join-Path $cache 'e5-models'
    $compactE5 = Join-Path $cache 'e5-compact'
    New-Item -ItemType Directory -Force -Path $cache, $models, $fullE5 | Out-Null

    function Get-VerifiedModel([string] $RelativeUrl, [string] $Name, [string] $Hash) {
        $file = Join-Path $fullE5 $Name
        if (!(Test-Path -LiteralPath $file) -or (Get-FileHash -LiteralPath $file).Hash -ne $Hash) {
            $url = "$($ModelSource.TrimEnd('/'))/Xenova/multilingual-e5-small/resolve/main/$RelativeUrl"
            curl.exe --fail --location --silent --show-error --max-time 240 --output "$file.download" $url
            if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath "$file.download").Hash -ne $Hash) {
                throw "Cannot verify model asset $Name"
            }
            Move-Item -Force -LiteralPath "$file.download" -Destination $file
        }
    }

    Get-VerifiedModel 'onnx/model_int8.onnx' 'e5-small-int8.onnx' '4D24E2BC01A447951524466EF533E52944BF48509E6552810BCEE1A2711CB02C'
    Get-VerifiedModel 'tokenizer.json' 'tokenizer.json' '0B44A9D7B51C3C62626640CDA0E2C2F70FDACDC25BBBD68038369D14EBDF4C39'
    $modelDev = Join-Path $cache 'model-dev'
    if (!(Test-Path -LiteralPath (Join-Path $modelDev 'onnx'))) {
        python -m pip install --disable-pip-version-check --only-binary=:all: --target $modelDev onnx==1.18.0
        if ($LASTEXITCODE -ne 0) { throw 'Could not install model build tools' }
    }
    $oldPythonPath = $env:PYTHONPATH
    try {
        $env:PYTHONPATH = $modelDev
        python scripts/compact-e5.py $fullE5 $compactE5
        if ($LASTEXITCODE -ne 0) { throw 'Could not compact the E5 model' }
    } finally {
        $env:PYTHONPATH = $oldPythonPath
    }
    $wheel = Join-Path $cache 'onnxruntime-1.22.0-cp312-cp312-win_amd64.whl'
    if (!(Test-Path -LiteralPath $wheel)) {
        python -m pip download --no-deps --only-binary=:all: --platform win_amd64 --python-version 3.12 --implementation cp --abi cp312 --dest $cache onnxruntime==1.22.0
        if ($LASTEXITCODE -ne 0) { throw 'ONNX Runtime download failed' }
    }
    if ((Get-FileHash -LiteralPath $wheel).Hash -ne 'C0D534A43D1264D1273C2D4F00A5A588FA98D21117A3345B7104FA0BBCAADB9A') {
        throw 'Unexpected ONNX Runtime archive hash'
    }
    $runtime = Join-Path $cache 'ort-wheel/onnxruntime'
    python -m zipfile -e $wheel (Split-Path -Parent $runtime)
    if ($LASTEXITCODE -ne 0) { throw 'Could not extract ONNX Runtime' }
    Copy-Item -LiteralPath (Join-Path $runtime 'capi/onnxruntime.dll'), (Join-Path $runtime 'capi/onnxruntime_providers_shared.dll') -Destination $models -Force

    if (!$SkipBuild) {
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    }

    $bundle = Join-Path $root "dist/$BundleName"
    $bundleModels = Join-Path $bundle 'models'
    $notices = Join-Path $bundle 'notices'
    New-Item -ItemType Directory -Force -Path $bundle, $bundleModels, $notices | Out-Null
    Copy-Item -LiteralPath 'target/release/tabsort-lite.exe' -Destination (Join-Path $bundle 'TabSortLite.exe') -Force
    Copy-Item -LiteralPath 'README.md' -Destination $bundle -Force
    Copy-Item -LiteralPath 'extension' -Destination $bundle -Recurse -Force
    foreach ($asset in 'onnxruntime.dll', 'onnxruntime_providers_shared.dll') {
        Copy-Item -LiteralPath (Join-Path $models $asset) -Destination $bundleModels -Force
    }
    foreach ($asset in 'e5-small-int8.onnx', 'tokenizer.json') {
        Copy-Item -LiteralPath (Join-Path $compactE5 $asset) -Destination $bundleModels -Force
    }
    Copy-Item -LiteralPath (Join-Path $runtime 'LICENSE') -Destination (Join-Path $notices 'ONNXRuntime-LICENSE.txt') -Force
    Copy-Item -LiteralPath (Join-Path $runtime 'ThirdPartyNotices.txt') -Destination $notices -Force
    Copy-Item -LiteralPath 'models/E5-NOTICE.txt' -Destination $notices -Force
    foreach ($obsolete in 'models/bge-small-zh-int8.onnx', 'notices/BGE-NOTICE.txt') {
        $oldAsset = Join-Path $bundle $obsolete
        if (Test-Path -LiteralPath $oldAsset -PathType Leaf) { Remove-Item -LiteralPath $oldAsset }
    }
    $zip = Join-Path $root "dist/$BundleName.zip"
    Compress-Archive -Path $bundle -DestinationPath $zip -Force
    $sourceZip = Join-Path $root "dist/$BundleName-Source.zip"
    $sourceDirectory = Join-Path $cache "$BundleName-Source"
    $sourceModels = Join-Path $sourceDirectory 'models'
    New-Item -ItemType Directory -Force -Path $sourceDirectory, $sourceModels | Out-Null
    Copy-Item -LiteralPath 'Cargo.toml', 'Cargo.lock', 'README.md', '.gitignore', 'src', 'extension', 'scripts' -Destination $sourceDirectory -Recurse -Force
    Copy-Item -LiteralPath 'models/E5-NOTICE.txt' -Destination $sourceModels -Force
    Compress-Archive -LiteralPath $sourceDirectory -DestinationPath $sourceZip -Force
    $artifacts = @($zip, $sourceZip, (Join-Path $bundle 'TabSortLite.exe'))
    $hashes = foreach ($artifact in $artifacts) {
        $hash = Get-FileHash -LiteralPath $artifact -Algorithm SHA256
        $relative = [IO.Path]::GetRelativePath((Join-Path $root 'dist'), $artifact).Replace('\', '/')
        "$($hash.Hash.ToLowerInvariant())  $relative"
    }
    $hashFile = Join-Path $root "dist/$BundleName-SHA256.txt"
    [IO.File]::WriteAllLines($hashFile, $hashes, [Text.UTF8Encoding]::new($false))
    Get-Item -LiteralPath $zip, $sourceZip, $hashFile
} finally {
    Pop-Location
}

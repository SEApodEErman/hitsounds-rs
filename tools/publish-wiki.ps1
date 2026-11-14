# Publishes wiki/*.md to the GitHub wiki of this repository.
#
#     powershell -ExecutionPolicy Bypass -File tools/publish-wiki.ps1
#
# GitHub creates the wiki's git repository only after its first page exists, so once,
# open the repository's Wiki tab and save any page; this script then replaces it.
# Uses your normal git credentials. Pages removed from wiki/ are removed from the wiki.

param([string]$Remote)

$ErrorActionPreference = 'Stop'
$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
if (-not $Remote) {
    $origin = git -C $Root remote get-url origin
    if (-not $origin) { throw 'No origin remote. Pass -Remote <wiki git URL>.' }
    $Remote = $origin -replace '\.git$', '' -replace '$', '.wiki.git'
}

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("hitsounds-rs-wiki-" + [guid]::NewGuid())
try {
    git clone --quiet $Remote $Work
    if ($LASTEXITCODE -ne 0) { throw "Could not clone $Remote. Create the first wiki page on GitHub, then run this again." }
    Get-ChildItem $Work -Filter *.md | Remove-Item
    Copy-Item (Join-Path $Root 'wiki\*.md') $Work
    git -C $Work add -A
    git -C $Work diff --cached --quiet
    if ($LASTEXITCODE -eq 0) { Write-Host 'The wiki is already up to date.'; return }
    $commit = git -C $Root rev-parse --short HEAD
    # Commit as this repository's identity, not the global one: the wiki is public, and
    # the repository may use a no-reply address the global config doesn't.
    $name = git -C $Root config user.name
    $email = git -C $Root config user.email
    git -C $Work -c "user.name=$name" -c "user.email=$email" commit --quiet -m "Publish wiki from $commit"
    git -C $Work push --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Push failed.' }
    Write-Host "Published the wiki from $commit."
}
finally {
    if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
}

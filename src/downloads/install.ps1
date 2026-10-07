# Connects this Windows machine to an Agent Router. It installs the machine agent, which
# lets the router follow and steer the Claude Code sessions that run here, and starts it
# at logon. The router serves this script with its own address filled in below:
#
#   irm <router>/agent/install.ps1 | iex
#
# It asks for the router's password unless AGENT_ROUTER_PASSWORD holds it. Where VS
# Code's `code` command is installed it also installs the Agent Router extension, which
# lets the router type into the sessions VS Code starts; without it they can be followed.

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 draws a progress bar that slows a download many times over.
$ProgressPreference = 'SilentlyContinue'
# And may still offer only TLS 1.0 and 1.1.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Server = @SERVER@
# Windows on Arm runs the x64 build.
$Platform = 'win32-x64'

$dir = Join-Path ([IO.Path]::GetTempPath()) ('agent-router-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
$asked = $false
try {
    function Get-AgentFile([string] $Name) {
        $out = Join-Path $dir $Name
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$Server/agent/$Platform/$Name" -OutFile $out
        } catch {
            throw "could not download $Name for $Platform from ${Server}: $($_.Exception.Message)"
        }
        $out
    }

    Write-Host "Downloading the agent for $Platform from $Server"
    $exe = Get-AgentFile 'agent-router.exe'

    # Asked here rather than by the agent, so it is not echoed: the agent reads a Windows
    # console as plain text. It reaches the agent in its environment, never an argument,
    # and leaves this session's environment once the install is done.
    if (-not $env:AGENT_ROUTER_PASSWORD) {
        $secure = Read-Host -AsSecureString 'Router password'
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try {
            $env:AGENT_ROUTER_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
        } finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
        $asked = $true
    }
    # `agent install` copies itself into %USERPROFILE%\.agent-router\bin, so the copy here
    # can go.
    & $exe agent install --server $Server
    if ($LASTEXITCODE -ne 0) { throw "the agent was not installed (agent-router exited with $LASTEXITCODE)" }

    $byHand = "install $Server/agent/$Platform/agent-router.vsix from VS Code (Extensions, ..., Install from VSIX)"
    if (Get-Command code -ErrorAction SilentlyContinue) {
        Write-Host 'Installing the Agent Router extension into VS Code'
        $vsix = Get-AgentFile 'agent-router.vsix'
        & code --install-extension $vsix --force
        if ($LASTEXITCODE -ne 0) { Write-Warning "VS Code did not take the extension: $byHand" }
    } else {
        Write-Host "VS Code's ``code`` command was not found, so the extension was not installed. This machine's sessions can be followed; to type into the ones VS Code starts, $byHand."
    }
} finally {
    if ($asked) { Remove-Item Env:AGENT_ROUTER_PASSWORD -ErrorAction SilentlyContinue }
    Remove-Item -Recurse -Force -Path $dir -ErrorAction SilentlyContinue
}

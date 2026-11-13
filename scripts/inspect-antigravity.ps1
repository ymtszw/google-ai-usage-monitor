<#
.SYNOPSIS
    Antigravity および language_server.exe の診断・トラブルシューティング スクリプト

.DESCRIPTION
    実行中の Antigravity 本体プロセスおよび language_server.exe のプロセス情報（PID、PPID、PEB コマンドライン引数、
    リッスンしている TCP ポート、CSRF トークン）を網羅的に収集・表示し、ローカル Quota API エンドポイントとの疎通テストを行います。

.PARAMETER Json
    結果をフォーマットされたテキストではなく JSON 形式で出力します。他ツールや自動スクリプトからの連携用です。

.PARAMETER SkipApiCheck
    ローカル Quota API エンドポイントへの疎通確認リクエストをスキップします。

.PARAMETER Detailed
    プロセスの完全なコマンドライン引数を省略せずに全文字表示します。

.EXAMPLE
    .\scripts\inspect-antigravity.ps1
    対話形式で診断ダッシュボードを表示します。

.EXAMPLE
    .\scripts\inspect-antigravity.ps1 -Json | ConvertFrom-Json
    JSON 形式で出力します。
#>

[CmdletBinding()]
param(
    [switch]$Json,
    [switch]$SkipApiCheck,
    [switch]$Detailed
)

Set-StrictMode -Off

function Write-Badge {
    param(
        [string]$Level,
        [string]$Message
    )
    $color = switch ($Level) {
        "OK"    { "Green" }
        "INFO"  { "Cyan" }
        "WARN"  { "Yellow" }
        "ERROR" { "Red" }
        default { "White" }
    }
    Write-Host " [$Level] " -ForegroundColor $color -NoNewline
    Write-Host $Message
}

function Get-ProcessListeningPorts {
    param([int]$TargetPid)
    $ports = @()
    try {
        if (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue) {
            $conns = Get-NetTCPConnection -OwningProcess $TargetPid -State Listen -ErrorAction SilentlyContinue
            if ($conns) {
                $ports = @($conns | Select-Object -ExpandProperty LocalPort -Unique | Sort-Object)
            }
        }
    } catch {
        # Fallback to netstat
    }

    if ($ports.Count -eq 0) {
        try {
            $netstatLines = netstat -ano -p tcp 2>$null | Select-String "LISTENING\s+$TargetPid`$"
            foreach ($line in $netstatLines) {
                if ($line -match ":(\d+)\s+.*LISTENING") {
                    $p = [int]$matches[1]
                    if ($p -notin $ports) {
                        $ports += $p
                    }
                }
            }
            $ports = @($ports | Sort-Object)
        } catch {}
    }

    return $ports
}

function Test-QuotaEndpoint {
    param(
        [int]$Port,
        [string]$CsrfToken
    )
    $url = "http://127.0.0.1:$Port/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary"
    $headers = @{
        "X-Codeium-Csrf-Token" = $CsrfToken
        "Content-Type"         = "application/json"
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $response = Invoke-RestMethod -Uri $url -Method Post -Headers $headers -Body "{}" -TimeoutSec 2 -ErrorAction Stop
        $sw.Stop()
        return @{
            Success     = $true
            Port        = $Port
            ElapsedMs   = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
            GroupsCount = ($response.response.groups).Count
            RawResponse = $response.response
            Error       = $null
        }
    } catch {
        $sw.Stop()
        $status = $null
        if ($_.Exception.Response) {
            $status = [int]$_.Exception.Response.StatusCode
        }
        return @{
            Success     = $false
            Port        = $Port
            ElapsedMs   = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
            StatusCode  = $status
            Error       = $_.Exception.Message
        }
    }
}

# --- 1. プロセス情報の取得 ---
$wmiProcesses = Get-CimInstance Win32_Process | Where-Object {
    $_.Name -eq "Antigravity.exe" -or $_.Name -eq "language_server.exe"
}

$antigravityProcs = @($wmiProcesses | Where-Object { $_.Name -eq "Antigravity.exe" } | Sort-Object ProcessId)
$languageServerProcs = @($wmiProcesses | Where-Object { $_.Name -eq "language_server.exe" } | Sort-Object ProcessId)

$csrfRegex = [regex]'--csrf_token\s+([a-zA-Z0-9\-]+)'

# language_server プロセスの詳細解析
$lsDetails = @()
$primaryLs = $null

foreach ($proc in $languageServerProcs) {
    $cmd = $proc.CommandLine
    $csrf = $null
    if ($cmd -and $cmd -match '--csrf_token\s+([a-zA-Z0-9\-]+)') {
        $csrf = $matches[1]
    }

    $ports = Get-ProcessListeningPorts -TargetPid $proc.ProcessId

    $role = "Background/Worker"
    if ($csrf) {
        $role = "Primary Server (RPC/HTTP)"
    } elseif ($cmd -match 'multicall\s+schedule') {
        $role = "Scheduled Task Runner"
    } elseif ($cmd -match 'multicall') {
        $role = "Multicall Worker"
    }

    $item = [PSCustomObject]@{
        ProcessId       = $proc.ProcessId
        ParentProcessId = $proc.ParentProcessId
        Role            = $role
        HasCsrfToken    = [bool]$csrf
        CsrfToken       = $csrf
        ListeningPorts  = $ports
        CommandLine     = $cmd
    }

    if ($csrf -and -not $primaryLs) {
        $primaryLs = $item
    }

    $lsDetails += $item
}

# Antigravity.exe プロセスの詳細解析
$agDetails = @()
foreach ($proc in $antigravityProcs) {
    $cmd = $proc.CommandLine
    $type = "Main/Browser Process"
    if ($cmd -match '--type=renderer') {
        $type = "Renderer"
    } elseif ($cmd -match '--type=gpu-process') {
        $type = "GPU Process"
    } elseif ($cmd -match '--type=utility') {
        $type = "Utility"
    } elseif ($cmd -match '--type=crashpad-handler') {
        $type = "Crashpad Handler"
    }

    $agDetails += [PSCustomObject]@{
        ProcessId       = $proc.ProcessId
        ParentProcessId = $proc.ParentProcessId
        Type            = $type
        CommandLine     = $cmd
    }
}

# --- 2. Quota API 疎通確認 ---
$endpointResults = @()
$activeQuotaSummary = $null

if (-not $SkipApiCheck -and $primaryLs -and $primaryLs.ListeningPorts.Count -gt 0) {
    foreach ($port in $primaryLs.ListeningPorts) {
        $res = Test-QuotaEndpoint -Port $port -CsrfToken $primaryLs.CsrfToken
        $endpointResults += $res
        if ($res.Success -and -not $activeQuotaSummary) {
            $activeQuotaSummary = $res
        }
    }
}

# --- 3. 出力生成 (JSON or Console) ---

$diagnosticData = [PSCustomObject]@{
    Timestamp               = (Get-Date).ToString("o")
    AntigravityRunning      = ($antigravityProcs.Count -gt 0)
    AntigravityProcessCount = $antigravityProcs.Count
    LanguageServerRunning   = ($languageServerProcs.Count -gt 0)
    LanguageServerCount     = $languageServerProcs.Count
    PrimaryLanguageServer   = $primaryLs
    AntigravityProcesses    = $agDetails
    LanguageServerProcesses = $lsDetails
    EndpointChecks          = $endpointResults
    ActiveQuota             = if ($activeQuotaSummary) { $activeQuotaSummary.RawResponse } else { $null }
}

if ($Json) {
    $diagnosticData | ConvertTo-Json -Depth 6
    exit 0
}

# --- コンソール整形出力 ---
Write-Host ""
Write-Host "==================================================================" -ForegroundColor Cyan
Write-Host "   Antigravity & Language Server Diagnostic Inspector" -ForegroundColor Cyan
Write-Host "==================================================================" -ForegroundColor Cyan
Write-Host "日時: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Gray
Write-Host ""

# 1. Antigravity 本体プロセスセクション
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan
Write-Host " 1. Antigravity GUI プロセス (Antigravity.exe)" -ForegroundColor White
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan

if ($antigravityProcs.Count -eq 0) {
    Write-Badge "WARN" "Antigravity.exe プロセスが検出されませんでした（未起動または別ユーザー権限の可能性）。"
} else {
    Write-Badge "OK" "$($antigravityProcs.Count) 件の Antigravity.exe プロセスが実行中です。"
    
    $mainProcs = $agDetails | Where-Object { $_.Type -eq "Main/Browser Process" }
    $otherProcs = $agDetails | Where-Object { $_.Type -ne "Main/Browser Process" }
    
    foreach ($p in $mainProcs) {
        Write-Host ("   - PID: {0,-6} [PPID: {1,-6}] {2}" -f $p.ProcessId, $p.ParentProcessId, $p.Type) -ForegroundColor Green
        if ($Detailed) {
            Write-Host "     CMD: $($p.CommandLine)" -ForegroundColor DarkGray
        }
    }
    if ($otherProcs.Count -gt 0) {
        $grouped = $otherProcs | Group-Object Type
        $summaryParts = $grouped | ForEach-Object { "$($_.Name): $($_.Count)" }
        Write-Host "   - サブプロセス ($($otherProcs.Count) 件): $($summaryParts -join ', ')" -ForegroundColor Gray
    }
}

Write-Host ""

# 2. language_server.exe プロセスセクション
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan
Write-Host " 2. Language Server プロセス (language_server.exe)" -ForegroundColor White
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan

if ($languageServerProcs.Count -eq 0) {
    Write-Badge "ERROR" "language_server.exe が実行されていません。Antigravity の初期化が完了していないか終了しています。"
} else {
    Write-Badge "OK" "$($languageServerProcs.Count) 件の language_server.exe プロセスを検出しました。"
    
    foreach ($ls in $lsDetails) {
        $portsStr = if ($ls.ListeningPorts.Count -gt 0) { ($ls.ListeningPorts -join ", ") } else { "None" }
        $csrfStr = if ($ls.HasCsrfToken) {
            $token = $ls.CsrfToken
            if ($token.Length -gt 12 -and -not $Detailed) {
                $token.Substring(0, 8) + "..." + $token.Substring($token.Length - 4)
            } else {
                $token
            }
        } else {
            "N/A"
        }

        $lineColor = if ($ls.HasCsrfToken) { "Green" } else { "Gray" }
        Write-Host ("   - PID: {0,-6} [PPID: {1,-6}] 役割: {2}" -f $ls.ProcessId, $ls.ParentProcessId, $ls.Role) -ForegroundColor $lineColor
        Write-Host ("     TCP Listen: {0,-18} CSRF Token: {1}" -f $portsStr, $csrfStr) -ForegroundColor DarkGray

        if ($Detailed -and $ls.CommandLine) {
            Write-Host "     CMD: $($ls.CommandLine)" -ForegroundColor DarkGray
        }
    }
}

Write-Host ""

# 3. ローカル Quota API 疎通セクション
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan
Write-Host " 3. ローカル Quota API エンドポイント疎通確認" -ForegroundColor White
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan

if ($SkipApiCheck) {
    Write-Badge "INFO" "-SkipApiCheck が指定されたため、API 疎通テストをスキップしました。"
} elseif (-not $primaryLs) {
    Write-Badge "ERROR" "有効な CSRF トークンを持つプライマリ language_server.exe が見つからないため、API テストを実施できません。"
} elseif ($primaryLs.ListeningPorts.Count -eq 0) {
    Write-Badge "WARN" "プライマリ language_server.exe (PID: $($primaryLs.ProcessId)) にリッスンポートが見つかりません。"
} else {
    $hasSuccess = $false
    foreach ($res in $endpointResults) {
        if ($res.Success) {
            $hasSuccess = $true
            Write-Badge "OK" "Port $($res.Port) への疎通成功 (応答時間: $($res.ElapsedMs) ms, クォータグループ数: $($res.GroupsCount))"
        } else {
            $statusStr = if ($res.StatusCode) { "HTTP $($res.StatusCode)" } else { "Timeout / ConnectFail" }
            Write-Badge "WARN" "Port $($res.Port) 疎通不可 ($statusStr - $($res.Error))"
        }
    }

    if ($activeQuotaSummary -and $activeQuotaSummary.RawResponse) {
        Write-Host ""
        Write-Host "   [取得クォータの概要]" -ForegroundColor Yellow
        foreach ($group in $activeQuotaSummary.RawResponse.groups) {
            Write-Host "     Group: $($group.displayName)" -ForegroundColor Cyan
            foreach ($bucket in $group.buckets) {
                $pct = [math]::Round($bucket.remainingFraction * 100, 1)
                $reset = if ($bucket.resetTime) { " (Reset: $($bucket.resetTime))" } else { "" }
                Write-Host ("       - {0,-28}: {1,5}% remaining{2}" -f $bucket.displayName, $pct, $reset) -ForegroundColor White
            }
        }
    } elseif (-not $hasSuccess) {
        Write-Badge "ERROR" "どのポートからも有効な Quota 応答を得られませんでした。CSRF トークンの不一致または認証制限の可能性があります。"
    }
}

Write-Host ""

# 4. 総合診断サマリー
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan
Write-Host " 4. 総合診断サマリー" -ForegroundColor White
Write-Host "------------------------------------------------------------------" -ForegroundColor DarkCyan

if ($antigravityProcs.Count -gt 0 -and $primaryLs -and $activeQuotaSummary) {
    Write-Badge "OK" "すべてのコンポーネントが正常に動作しています。google-ai-usage-monitor との連携準備は完了しています。"
} elseif ($antigravityProcs.Count -eq 0) {
    Write-Badge "WARN" "Antigravity が起動していません。Antigravity を起動してから再度ご確認ください。"
} elseif (-not $primaryLs) {
    Write-Badge "WARN" "Antigravity は起動していますが、language_server.exe の CSRF トークンを検出できませんでした。"
} elseif ($SkipApiCheck) {
    Write-Badge "INFO" "プロセスの存在と CSRF トークンを確認しました（-SkipApiCheck のため API 疎通は未実施）。"
} elseif (-not $activeQuotaSummary) {
    Write-Badge "WARN" "language_server.exe は検出されましたが、Quota API への接続に失敗しました。"
}

Write-Host "==================================================================" -ForegroundColor Cyan
Write-Host ""

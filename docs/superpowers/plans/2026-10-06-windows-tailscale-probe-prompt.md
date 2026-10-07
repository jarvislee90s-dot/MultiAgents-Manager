# Windows 端 Tailscale 探测任务（发给 Windows Agent 执行）

> **你接手时的状态**：这台 Windows 机器上 **Tailscale 已经安装，并且已经登录了**。你要做的是**从当前状态继续往前探测**——把「已安装 / 已登录」的判据钉死，再把后面几步（关闭阻止传入连接 → 开通 Funnel → 验证地址可通 → 重启恢复）跑一遍，并记录每一处可观测的信号。
> **产出一张填好的表回传。**

---

## 给执行者的话（先读）

1. **不要跳步**，每步做完立刻记录原始输出。不要只写"成功了"——要写"**用哪条命令、看哪个字段、值是什么**"。
2. **每条命令都把退出码一起记**（PowerShell 里紧跟着跑 `$LASTEXITCODE`）。
3. **不要猜**：命令不存在、字段没有、行为与预期不符——**照实写"未观察到"**，这比编一个像样的答案有价值得多。
4. **安全**：第 4 节会把一个端口短暂暴露到公网。**只暴露一个没有内容的测试端口**，探测完立刻关闭（第 8 节）。
5. **全程不需要管理员权限**——Tailscale 的日常操作都不需要提权。（唯一一次提权发生在软件安装时，那已经过去了。）
6. **本机自测不算数**：第 5 节**必须从这台机器外面验证**——原因见那一节。

---

## 1. ⭐ 现状与「已安装 / 已登录」的判据

**这一节是整个探测的基础**：MAM 的引导界面要靠这些判断"用户装好了没有、登录了没有"。请逐条记录**原样输出**。

### 1.1 已安装的判据（现在就能观测）

```powershell
# ① CLI 的确切路径（MAM 靠这个路径找它）
Test-Path "C:\Program Files\Tailscale\tailscale.exe"
Get-Command tailscale -ErrorAction SilentlyContinue | Select-Object Source

# ② 服务名与启动类型
Get-Service -Name Tailscale -ErrorAction SilentlyContinue | Select-Object Name,Status,StartType

# ③ 卸载登记项（"已安装"最可靠的判据之一）
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*" |
  Where-Object { $_.DisplayName -like "*Tailscale*" } |
  Select-Object DisplayName, DisplayVersion, InstallLocation, Publisher

# ④ 版本
& "C:\Program Files\Tailscale\tailscale.exe" version
```

> **要记录**：`tailscale.exe` 的**确切路径**（是不是 `C:\Program Files\Tailscale\tailscale.exe`？）；**服务名**是否就叫 `Tailscale`、启动类型是什么；**卸载登记项里的 `DisplayName` / `DisplayVersion` / `InstallLocation` / `Publisher` 具体值**；`tailscale version` 的输出格式。
> 额外一问：从 `DisplayVersion` 能看出装的是 **MSI 还是 exe 版**吗？版本号是多少？

### 1.2 已登录的判据（现在就能观测）

```powershell
$TS = "C:\Program Files\Tailscale\tailscale.exe"
& $TS status --json
```

> **要记录**：`BackendState` 的值是什么（预期是 `"Running"`）？`Self.DNSName` 与 `CertDomains` 的值是什么形态？

### 1.3 架构与系统版本

```powershell
[System.Environment]::Is64BitOperatingSystem
$env:PROCESSOR_ARCHITECTURE          # AMD64 / ARM64 / x86
[System.Environment]::OSVersion.Version
```

---

## 2. ⭐ 字段核对表（本任务**最重要**的部分）

下面的命令与字段是 **MAM 在 macOS 上实测确认过**的——MAM 的引导界面**正是靠读这些来判断每一步做完了没有**。**Windows 上可能不一样，请逐条核对**，把你看到的**原样**填进表里。

先把输出存下来（后面要用）：

```powershell
$TS = "C:\Program Files\Tailscale\tailscale.exe"
& $TS status --json        | Out-File -Encoding utf8 "$env:TEMP\ts-status.json"
& $TS get --json           | Out-File -Encoding utf8 "$env:TEMP\ts-get.json"
& $TS funnel status --json | Out-File -Encoding utf8 "$env:TEMP\ts-funnel.json"
```

| #   | MAM 在 macOS 上读什么              | macOS 上的实测值                                  | **Windows 上是什么？** |
| --- | ---------------------------------- | ------------------------------------------------- | ---------------------- |
| 1   | `status --json` → `BackendState`   | `"Running"`（已登录在线）                         | ？                     |
| 2   | `status --json` → `AuthURL`        | 已登录时是**空串**；待登录时带授权链接            | ？                     |
| 3   | `status --json` → `Self.DNSName`   | **带结尾点**（`xxx.ts.net.`）——MAM 必须去掉这个点 | ？**带点吗？**         |
| 4   | `status --json` → `CertDomains[0]` | 同上一行但**不带**结尾点                          | ？                     |
| 5   | `funnel status --json`             | **未开通时是空对象 `{}`**                         | ？未开通时是什么？     |
| 6   | `get --json` → 键名                | 精确叫 **`shields-up`**，值是布尔                 | ？**键名一样吗？**     |
| 7   | `tailscale up` 的行为              | 未登录时会给出/打开授权链接                       | ？                     |

> **为什么这张表最要紧**：MAM 的代码是照 macOS 写的。**只要某条字段在 Windows 上换了名字或换了形态**（例如 `Self.DNSName` 不带了结尾点、或者那个偏好项不叫 `shields-up`），**引导界面就会在 Windows 上判断错误**——可能把"没装好"当成"装好了"，或者卡在某一步不动。
> 顺带：如果某条命令在 Windows 上**根本不存在**，也请明确写出来。

---

## 3. 关闭「阻止传入连接」（shields-up）

**背景**：Tailscale 有个叫 **shields-up**（阻止传入连接）的偏好项，开着的话 Funnel 不会被外部访问到。macOS 上**读取**已实测，**写入从未实测过**——这条你正好能补上。

```powershell
$TS = "C:\Program Files\Tailscale\tailscale.exe"

# ① 读：这个偏好项在本机叫什么、当前值是什么
& $TS get --json ; $LASTEXITCODE

# ② 写：程序化关闭它
& $TS set --shields-up=false ; $LASTEXITCODE

# ③ 回读确认真的关了
& $TS get --json ; $LASTEXITCODE
```

> **要记录**：① 字段的**确切名字**（是 `shields-up` 还是别的写法？值是真/假还是字符串？）；② `set --shields-up=false` **能不能用**、退出码多少、**回读是否真的变了**；③ 需不需要管理员权限。

---

## 4. 开启 Funnel（首次会要求一次浏览器批准）

```powershell
$TS = "C:\Program Files\Tailscale\tailscale.exe"

# ① 先起一个「没有任何内容」的测试端口（另开一个 PowerShell 窗口跑）
#    例：python -m http.server 19999     （或任何你方便的、无内容的本地服务）

# ② 开通 Funnel（把本地 19999 暴露出去）
& $TS funnel --bg 19999 ; $LASTEXITCODE

# ③ 看状态
& $TS funnel status --json ; $LASTEXITCODE
```

> **要记录**：
>
> - `funnel --bg` 的**完整输出**——它有没有打印一个**需要点开的批准链接**？链接在哪？
> - **是否必须去浏览器点一次"批准"**？批准页面的实际文案是什么？
> - 批准**前后** `funnel status --json` 的**差异**（哪些字段变了？）——请把两次输出都留下。
> - 稳定之后，**公网地址**在哪个字段？把地址原文记下来（**这一条是 MAM 要显示给你的地址**）。
> - **`--bg` 是不是必需**？（不加会怎样？会阻塞终端吗？）

---

## 5. 验证地址真的能通（**这一步不能在这台机器上做**）

> **为什么**：**在这台机器上测是测不出问题的**。两个原因：① 这台机器自己就在这个网络里，解析那个域名时会被**本地接管**，**永远显示通**；② 机器上可能装了网络代理/加速工具，会拦截请求。
> **我们实测中就是被这一点骗过一次**：本机所有检查全绿（连 HTTPS 证书都真签下来了），但公网根本打不开。

所以**必须从外面测**，用公共解析器（绕过本机 DNS）：

```powershell
# ① 从公网 DNS 解析（指定公共解析器，别用系统默认）
Resolve-DnsName -Name "<你的固定地址域名>" -Server 223.5.5.5 -Type A

# ② 从外面尝试接通
curl.exe -sS -o NUL -w "HTTP %{http_code}`n" "https://<你的固定地址域名>/"

# ③ 最好再用手机（切到蜂窝数据，别连 WiFi）打开一次那个地址，看能不能看到东西
```

> **要记录**：
>
> - 公共解析器**能不能解析出 A 记录**？值是什么？
> - 直接请求返回的 **HTTP 状态码**；
> - **首次开通后有没有出现"解析不出来 / 要等一会儿才通"**？等了多久？（MAM 专门要处理这一类故障：新增通道时解析记录需要一段时间才生效）
> - 如果一开始不通：**关掉再重开**（先 `funnel reset` 再 `funnel --bg 19999`）能不能修好？
> - 第 ③ 步（手机蜂窝）**成功还是失败**？——这是**唯一**能证明"真能从外面连上"的证据。

---

## 6. 重启后能否自动恢复

```powershell
# 重启这台机器，然后：
$TS = "C:\Program Files\Tailscale\tailscale.exe"
Get-Service -Name Tailscale | Select-Object Name,Status
& $TS status --json          # BackendState 是什么？
& $TS funnel status --json   # Funnel 配置还在吗？
```

> **要记录**：重启后**服务是否自动起来**；**登录态是否保持**；**Funnel 配置是否保留**（还是要重新开一次/重新批准）；**地址是否与重启前完全一致**。

---

## 7. 写路径验证（MAM 会执行这些动作，但它们从未在 Windows 上跑过）

MAM 会程序化执行下面三个动作。请确认它们在 Windows 上**能不能用**：

| 动作       | 命令                               | 要记录                                                                                                                                          |
| ---------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 开通       | `tailscale funnel --bg <端口>`     | 退出码 / 是否阻塞 / 是否立即返回                                                                                                                |
| 撤销       | `tailscale funnel reset`           | 退出码 / 撤销后 `funnel status --json` 变成什么（预期是 `{}`）/ **会不会连带清掉别的 serve 配置**（若你自己配过 `tailscale serve`，请特别留意） |
| 关闭偏好项 | `tailscale set --shields-up=false` | 见第 3 节                                                                                                                                       |

> 这三个动作在 macOS 上也是**没有实测过**的，所以这里的结论**同样珍贵**。

---

## 8. 收尾（必做）

```powershell
$TS = "C:\Program Files\Tailscale\tailscale.exe"

# ① 关闭 Funnel（不要留着公网暴露）
& $TS funnel reset
& $TS funnel status     # 预期：No serve config

# ② 停掉测试用的 HTTP 端口（关掉那个 PowerShell 窗口即可）

# ③ 报告最终状态
& $TS status --json | Select-String "BackendState"
```

> **不要卸载 Tailscale**（这台机器后续还要用）。**不要**留着 Funnel 开启。

---

## 9. 回填表（请把这张表填满交回）

| #   | 步骤                         | **怎么判断这一步完成了**（确切命令 + 字段 + 值） | 需要人工点击吗 | 意外情况                                                         |
| --- | ---------------------------- | ------------------------------------------------ | -------------- | ---------------------------------------------------------------- |
| 1.1 | 是否已安装                   |                                                  | 否             | CLI 路径：____ ｜ 服务名：____ ｜ 版本：____                     |
| 1.2 | 是否已登录                   |                                                  | 否             | `BackendState`：____                                             |
| 3   | 关闭 shields-up              |                                                  | 否             | 字段名：______ ｜ `set` 可用？______                             |
| 4   | 开启 Funnel                  |                                                  | 是（首次批准） | 批准链接从哪来：______ ｜ 地址字段：______                       |
| 5   | 地址可通                     |                                                  | 否             | 公网解析：______ ｜ HTTP 码：______ ｜ 手机蜂窝：成功/失败       |
| 6   | 重启恢复                     |                                                  | 否             | 服务自启：____ ｜ 登录态：____ ｜ Funnel：____ ｜ 地址不变：____ |
| 7   | 写路径（开通/撤销/关闭偏好） |                                                  | 否             | 三条各自的退出码与副作用                                         |

**四个必须回答的问题：**

1. **在你这次探测涉及的步骤里，哪几步需要人工点击？分别是什么？**（MAM 的目标是把它们压到最少，所以要数清楚）
2. **哪一步最可能卡住用户？** 你实际体验下来，哪一步最关键、最容易让人放弃？
3. **有没有出现上面没写到的额外步骤或意外？**（防火墙提示、杀毒拦截、需要重启、UAC 二次弹窗……**这些必须记下来**，否则 MAM 的引导会漏步）
4. **第 2 节字段核对表里，有没有哪一条与 macOS 不一样？**（这是最关键的产出）

---

## 附：MAM 目标流程（对照参考，**不要照抄**）

MAM 要在 Windows 上把这个流程做成"点几下就完成"，你的探测结果就是它的判据来源：

| #   | 步骤                         | 谁做                       | 用户动作                    |
| --- | ---------------------------- | -------------------------- | --------------------------- |
| 1   | 检测是否已安装               | 程序                       | 无                          |
| 2   | 从官方源下载并校验           | 程序                       | 无                          |
| 3   | 触发安装                     | 程序触发，系统弹管理员确认 | **输一次系统密码 / 点 UAC** |
| 4   | **（仅 macOS）批准系统扩展** | 系统弹出，用户去系统设置   | **点一次「允许」**          |
| 5   | 登录 Tailscale               | 用户在浏览器完成           | **点一次登录**              |
| 6   | 关闭「阻止传入连接」         | 程序                       | 无                          |
| 7   | 开启 Funnel                  | 程序触发，浏览器弹一次批准 | **点一次「批准」**          |
| 8   | 验证并显示固定地址           | 程序                       | 无                          |
| 9   | 此后每次开机                 | 程序自动恢复               | 零维护                      |

> 本次探测从**第 6 步**开始（第 1–5 步在这台机器上已经完成了），但第 1 节的观测结果同样重要——它是"已安装 / 已登录"的判据。

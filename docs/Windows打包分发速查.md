# Windows 打包分发速查（config-api）

把服务拷到另一台 Windows 机器上跑，照这份清单做即可。目标机器**不需要装 Rust / Python / JVM**。

## 1. 取产物

| 命令 | 产物 | 用于分发 |
|---|---|---|
| `cargo build` | `target\debug\config-api.exe` | 否，体积大、带调试信息 |
| `cargo build --release` | `target\release\config-api.exe` | **是**，体积约为 debug 的 1/7 |

`cargo run` 跑的也是 debug 版本，别拿它分发。

## 2. 打包结构

```text
config-api/
├── config-api.exe          ← target\release\config-api.exe
├── config.toml             ← 项目根目录那份
└── data/
    ├── SmartViewConfig.txt
    └── downloads/          ← 空目录也要建
```

**为什么这三个要在一起**：`config.toml` 里的 `./data`、`./data/downloads` 是相对 **config.toml 所在目录**解析的，不是相对 exe，也不是相对双击时的工作目录。Swagger UI 是 `vendored` 编进 exe 的，不用拷网页文件。

启动时会校验 `base_dir`、目标 txt、下载目录（`download.enabled = true` 时）是否存在，缺了会直接退出并打印原因。

## 3. 目标机器上

```powershell
cd D:\config-api
.\config-api.exe
```

双击 exe 也行。验证：

```powershell
curl http://127.0.0.1:18981/api/config
```

当前 `config.toml` 是 `host = "0.0.0.0"`，局域网其他机器可访问，记得防火墙放行 TCP `18981`。只给本机用就改回 `127.0.0.1`。

## 4. 最容易踩的坑：VC++ 运行库

本机工具链是 `x86_64-pc-windows-msvc`，默认**动态链接 CRT**。目标机如果是裸机，双击可能报缺 `VCRUNTIME140.dll` 或 `api-ms-win-crt-*.dll`。二选一：

**方案 A：目标机装运行库**

装 Visual C++ 2015-2022 Redistributable (x64)。

**方案 B：编译成静态 CRT（拷过去就能跑）**

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --target x86_64-pc-windows-msvc
```

取 `target\x86_64-pc-windows-msvc\release\config-api.exe` 分发。

> 未签名的 exe 可能被杀毒软件误报，企业内部分发建议做代码签名。

## 5. 配置文件放别处

程序接受第一个命令行参数指定配置路径：

```powershell
.\config-api.exe D:\config-api\config-test.toml
```

此时 `./data` 等相对路径以**这个 toml 所在目录**为基准。

## 6. 启动失败自查

| 现象 | 原因 |
|---|---|
| 提示 `base_dir` 不存在 | 路径写错，Windows 反斜杠要双写：`C:\\SmartView\\config` |
| 提示 `download.roots` 不存在 | 下载目录没建 |
| `Address already in use` | 端口被占用，改 `server.port` |
| 双击闪退 | 多半是缺 VC++ 运行库，见第 4 节 |
| 局域网访问不到 | `host` 不是内网 IP / `0.0.0.0`，或防火墙没放行 |

更完整的配置和接口说明见 [`README.zh-CN.md`](../README.zh-CN.md)，代码级打包矩阵见 [`Axum配置接口_完整实现与打包指南.md`](./Axum配置接口_完整实现与打包指南.md)。

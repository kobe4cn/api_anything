#!/usr/bin/env bash
set -euo pipefail

# ============================================================
# API-Anything E2E 全链路自动化测试运行器
# 按模块逐个执行测试并汇总结果，生成 Markdown 和 JSON 两种格式报告
# ============================================================

REPORT_DIR="test-reports/$(date +%Y%m%d_%H%M%S)"
mkdir -p "$REPORT_DIR"

export DATABASE_URL="${DATABASE_URL:-postgres://api_anything:api_anything@localhost:5432/api_anything}"
export RUST_LOG=warn
export RUST_BACKTRACE=1

echo "======================================"
echo "API-Anything E2E 测试套件"
echo "时间: $(date '+%Y-%m-%d %H:%M:%S')"
echo "数据库: $DATABASE_URL"
echo "报告目录: $REPORT_DIR"
echo "======================================"

# 测试模块列表：名称:crate 名
MODULES=(
    "common crate 单元测试:api-anything-common"
    "gateway 单元测试:api-anything-gateway"
    "generator 单元测试:api-anything-generator"
    "sandbox 单元测试:api-anything-sandbox"
    "compensation 单元测试:api-anything-compensation"
    "metadata 集成测试:api-anything-metadata"
    "platform-api 集成测试:api-anything-platform-api"
    "CLI 测试:api-anything-cli"
)

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0
RESULTS=()

for MODULE in "${MODULES[@]}"; do
    IFS=':' read -r NAME CRATE <<< "$MODULE"
    echo ""
    echo "--------------------------------------"
    echo "运行: $NAME ($CRATE)"
    echo "--------------------------------------"

    OUTPUT_FILE="$REPORT_DIR/${CRATE}.txt"

    START=$(date +%s)
    if cargo test -p "$CRATE" 2>&1 | tee "$OUTPUT_FILE"; then
        STATUS="PASS"
    else
        STATUS="FAIL"
    fi
    END=$(date +%s)
    DURATION=$(( END - START ))

    # 从 cargo test 输出中提取统计数字
    PASS=$(grep -o '[0-9]* passed' "$OUTPUT_FILE" | awk '{sum+=$1} END {print sum+0}')
    FAIL=$(grep -o '[0-9]* failed' "$OUTPUT_FILE" | awk '{sum+=$1} END {print sum+0}')
    SKIP=$(grep -o '[0-9]* ignored' "$OUTPUT_FILE" | awk '{sum+=$1} END {print sum+0}')

    TOTAL_PASS=$((TOTAL_PASS + PASS))
    TOTAL_FAIL=$((TOTAL_FAIL + FAIL))
    TOTAL_SKIP=$((TOTAL_SKIP + SKIP))

    RESULTS+=("$NAME|$PASS|$FAIL|$SKIP|${DURATION}s|$STATUS")

    echo "  结果: $PASS passed, $FAIL failed, $SKIP ignored (${DURATION}s)"
done

# ── 计算通过率 ────────────────────────────────────────────────────────
TOTAL=$((TOTAL_PASS + TOTAL_FAIL + TOTAL_SKIP))
if [ "$TOTAL_PASS" -eq 0 ] && [ "$TOTAL_FAIL" -eq 0 ]; then
    PASS_RATE="N/A"
    PASS_RATE_NUM=0
else
    PASS_RATE_NUM=$(echo "scale=1; $TOTAL_PASS * 100 / ($TOTAL_PASS + $TOTAL_FAIL)" | bc)
    PASS_RATE="${PASS_RATE_NUM}%"
fi

# ── 生成 Markdown 报告 ───────────────────────────────────────────────
REPORT="$REPORT_DIR/test-report.md"
RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}' || echo "N/A")
PG_VERSION=$(psql --version 2>/dev/null | head -1 || echo "N/A")

cat > "$REPORT" << HEADER
# API-Anything 全链路测试报告

**执行时间:** $(date '+%Y-%m-%d %H:%M:%S')
**环境:** $(uname -s) $(uname -m) | Rust ${RUST_VERSION}
**数据库:** ${PG_VERSION}

---

## 总体结果

| 指标 | 数值 |
|------|------|
| 总测试数 | ${TOTAL} |
| 通过 | ${TOTAL_PASS} |
| 失败 | ${TOTAL_FAIL} |
| 跳过 | ${TOTAL_SKIP} |
| 通过率 | ${PASS_RATE} |

## 模块详情

| 模块 | 通过 | 失败 | 跳过 | 耗时 | 状态 |
|------|------|------|------|------|------|
HEADER

for RESULT in "${RESULTS[@]}"; do
    IFS='|' read -r NAME PASS FAIL SKIP DUR STATUS <<< "$RESULT"
    if [ "$STATUS" = "PASS" ]; then
        ICON="PASS"
    else
        ICON="FAIL"
    fi
    echo "| $NAME | $PASS | $FAIL | $SKIP | $DUR | $ICON |" >> "$REPORT"
done

cat >> "$REPORT" << 'FOOTER'

## 测试覆盖范围

### 协议适配器
- [x] SOAP (WSDL 解析 -> XML<->JSON -> 网关代理)
- [x] CLI (help 解析 -> tokio::process -> 输出解析)
- [x] SSH (样例解析 -> 系统 ssh -> 输出解析)
- [x] PTY (Expect 状态机 -> stdin/stdout pipe)

### 保护层
- [x] 令牌桶限流 (burst -> refill -> reject)
- [x] 滑动窗口熔断 (Closed -> Open -> HalfOpen -> Closed)
- [x] 并发信号量 (acquire -> release -> limit)

### 沙箱
- [x] Mock (Schema 生成 -> Smart Mock -> Fixed Response)
- [x] Replay (录制 -> 匹配 -> 回放)
- [x] Proxy (转发 -> 租户隔离 -> 只读模式)

### 补偿机制
- [x] at_most_once (不记录)
- [x] at_least_once (记录 -> 重试)
- [x] exactly_once (幂等键 -> 去重)
- [x] 死信管理 (查看 -> 重推 -> 标记)

### 安全
- [x] 命令注入防护 (7 种注入变体)
- [x] RFC 7807 错误格式
- [x] 敏感信息泄露检查

### 文档服务
- [x] OpenAPI 3.0 动态生成
- [x] Swagger UI
- [x] Agent Prompt

## 过程数据分析

### 测试执行时间分布
(详见各模块 .txt 日志文件)

### 发现的问题
(如有失败测试，详情见对应模块日志)

---
*报告由 API-Anything E2E 测试套件自动生成*
FOOTER

echo ""
echo "======================================"
echo "测试执行完成"
echo "  通过: $TOTAL_PASS"
echo "  失败: $TOTAL_FAIL"
echo "  跳过: $TOTAL_SKIP"
echo "  报告: $REPORT"
echo "======================================"

# ── 生成 JSON 格式报告（供 CI 解析） ──────────────────────────────────
JSON_REPORT="$REPORT_DIR/test-results.json"

# 先构建 modules 数组的 JSON 内容
MODULES_JSON=""
FIRST=true
for RESULT in "${RESULTS[@]}"; do
    IFS='|' read -r NAME PASS FAIL SKIP DUR STATUS <<< "$RESULT"
    if [ "$FIRST" = true ]; then
        FIRST=false
    else
        MODULES_JSON="${MODULES_JSON},"
    fi
    MODULES_JSON="${MODULES_JSON}
    {\"name\": \"${NAME}\", \"passed\": ${PASS}, \"failed\": ${FAIL}, \"skipped\": ${SKIP}, \"duration\": \"${DUR}\", \"status\": \"${STATUS}\"}"
done

cat > "$JSON_REPORT" << ENDJSON
{
  "timestamp": "$(date -u '+%Y-%m-%dT%H:%M:%SZ')",
  "total_tests": ${TOTAL},
  "passed": ${TOTAL_PASS},
  "failed": ${TOTAL_FAIL},
  "skipped": ${TOTAL_SKIP},
  "pass_rate": ${PASS_RATE_NUM},
  "modules": [${MODULES_JSON}
  ]
}
ENDJSON

echo "  JSON 报告: $JSON_REPORT"

# 如果有失败测试，返回非零退出码通知 CI
[ "$TOTAL_FAIL" -eq 0 ] || exit 1

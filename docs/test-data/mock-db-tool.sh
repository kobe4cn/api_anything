#!/bin/bash
# 模拟数据库管理工具 dbctl，用于手工测试 CLI 类型的 API 生成与网关代理。
# 覆盖 query / status / backup / users 四个子命令，分别测试 JSON 输出、
# 文本表格输出、带退出码的失败场景。

case "$1" in
    query)
        shift
        sql=""
        format="json"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --format) format="$2"; shift 2 ;;
                --read-only) shift ;;
                --limit) shift 2 ;;
                *) sql="$1"; shift ;;
            esac
        done

        if [[ -z "$sql" ]]; then
            echo "Error: SQL query is required" >&2
            exit 1
        fi

        if [[ "$format" == "json" ]]; then
            cat <<'JSONEOF'
{
  "columns": ["id", "name", "email", "created_at"],
  "rows": [
    {"id": 1, "name": "Alice", "email": "alice@example.com", "created_at": "2024-01-15T10:30:00Z"},
    {"id": 2, "name": "Bob", "email": "bob@example.com", "created_at": "2024-02-20T14:15:00Z"},
    {"id": 3, "name": "Charlie", "email": "charlie@example.com", "created_at": "2024-03-10T09:45:00Z"}
  ],
  "row_count": 3,
  "execution_time_ms": 12
}
JSONEOF
        else
            cat <<'TABLEEOF'
 id | name    | email               | created_at
----+---------+---------------------+------------------------
  1 | Alice   | alice@example.com   | 2024-01-15T10:30:00Z
  2 | Bob     | bob@example.com     | 2024-02-20T14:15:00Z
  3 | Charlie | charlie@example.com | 2024-03-10T09:45:00Z
(3 rows)
TABLEEOF
        fi
        ;;

    status)
        cat <<'STATUSEOF'
{
  "host": "localhost",
  "port": 5432,
  "version": "PostgreSQL 16.2",
  "uptime_seconds": 864000,
  "connections": {
    "active": 15,
    "idle": 5,
    "max": 100
  },
  "database_size_mb": 2048,
  "replication": {
    "role": "primary",
    "replicas": 2,
    "lag_bytes": 0
  }
}
STATUSEOF
        ;;

    backup)
        shift
        db_name=""
        output=""
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --database|-d) db_name="$2"; shift 2 ;;
                --output|-o) output="$2"; shift 2 ;;
                *) shift ;;
            esac
        done

        if [[ -z "$db_name" ]]; then
            echo "Error: --database is required for backup" >&2
            exit 1
        fi

        echo "{\"backup_id\": \"BK-$(date +%Y%m%d%H%M%S)\", \"database\": \"$db_name\", \"size_mb\": 512, \"status\": \"completed\", \"path\": \"${output:-/tmp/backup.sql.gz}\"}"
        ;;

    users)
        shift
        case "$1" in
            list)
                echo '[{"username": "admin", "role": "superuser", "connections": 3}, {"username": "app_user", "role": "readwrite", "connections": 12}, {"username": "readonly", "role": "readonly", "connections": 0}]'
                ;;
            create)
                echo '{"username": "new_user", "role": "readwrite", "created": true}'
                ;;
            *)
                echo "Unknown users subcommand: $1" >&2
                exit 1
                ;;
        esac
        ;;

    *)
        echo "dbctl: unknown command '$1'" >&2
        echo "Run 'dbctl --help' for usage" >&2
        exit 1
        ;;
esac

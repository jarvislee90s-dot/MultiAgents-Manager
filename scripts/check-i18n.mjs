#!/usr/bin/env node
// 校验 zh/en 语言文件键集一致；不一致时列出差异并以非零码退出
import { readFileSync } from "node:fs";

const flat = (obj, prefix = "") =>
  Object.entries(obj).flatMap(([k, v]) =>
    typeof v === "object" && v !== null ? flat(v, `${prefix}${k}.`) : [`${prefix}${k}`]
  );

const zh = JSON.parse(readFileSync("src/i18n/locales/zh.json", "utf8"));
const en = JSON.parse(readFileSync("src/i18n/locales/en.json", "utf8"));
const zk = new Set(flat(zh));
const ek = new Set(flat(en));
const missEn = [...zk].filter((k) => !ek.has(k));
const missZh = [...ek].filter((k) => !zk.has(k));

if (missEn.length || missZh.length) {
  console.error(`i18n 键不一致: en 缺 ${missEn.length} 个:`, missEn);
  console.error(`i18n 键不一致: zh 缺 ${missZh.length} 个:`, missZh);
  process.exit(1);
}
console.log(`i18n 键对齐通过（${zk.size} 键）`);

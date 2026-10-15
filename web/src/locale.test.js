import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { chooseLocale, english, translate } from "./locale.js";

test("a supported saved choice wins; unknown preferences fall back to browser language", () => {
  assert.equal(chooseLocale("en", "zh-CN"), "en");
  assert.equal(chooseLocale("zh-CN", "en-US"), "zh-CN");
  assert.equal(chooseLocale("invalid", "zh-TW"), "zh-CN");
  assert.equal(chooseLocale(null, "en-GB"), "en");
  assert.equal(chooseLocale(null, "fr-FR"), "en");
  assert.equal(chooseLocale(undefined, undefined), "en");
});

test("notices translate at display time without rewriting operator-provided values", () => {
  const notice = { message: "新代次必须大于 {generation}。", values: { generation: 41 } };
  assert.equal(translate("en", notice), "The new generation must be greater than 41.");
  assert.equal(translate("zh-CN", notice), "新代次必须大于 41。");
  assert.equal(translate("en", "确定禁用 {id}？", { id: "实例-{generation}" }), "Disable 实例-{generation}?");
  assert.equal(translate("en", "upstream certificate verification failed"), "upstream certificate verification failed");
  assert.equal(translate("en", ""), "");
  assert.equal(translate("en", "constructor"), "constructor");
  assert.equal(translate("en", "__proto__"), "__proto__");
});

test("every console source message has English copy with the same interpolation fields", () => {
  const source = readFileSync(new URL("./main.jsx", import.meta.url), "utf8");
  const placeholders = (text) => [...new Set(text.match(/\{\w+\}/g) ?? [])].sort();
  for (const [message, translation] of Object.entries(english)) {
    assert.ok(translation.trim(), `Empty translation: ${message}`);
    assert.deepEqual(placeholders(translation), placeholders(message), `Changed fields: ${message}`);
    assert.doesNotMatch(translation, /\p{Script=Han}/u, `Untranslated English copy: ${message}`);
  }
  for (const [literal] of source.matchAll(/"(?:\\.|[^"\\])*"/g)) {
    const message = JSON.parse(literal);
    if (/\p{Script=Han}/u.test(message)) assert.ok(Object.hasOwn(english, message), `Missing translation: ${message}`);
  }
});

import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { chooseLocale, formatTime, translate } from "./locale.js";

const LanguageContext = createContext(null);
const STORAGE_KEY = "senix.language";

function initialLocale() {
  let saved;
  try { saved = window.localStorage.getItem(STORAGE_KEY); } catch { /* Storage can be disabled. */ }
  return chooseLocale(saved, navigator.languages?.[0] || navigator.language);
}

export function LanguageProvider({ children }) {
  const [locale, setLocale] = useState(initialLocale);
  useEffect(() => {
    document.documentElement.lang = locale;
    document.title = translate(locale, "Senix 控制台");
  }, [locale]);
  const value = useMemo(() => ({
    locale,
    setLocale: (next) => {
      if (next !== "zh-CN" && next !== "en") return;
      setLocale(next);
      try { window.localStorage.setItem(STORAGE_KEY, next); } catch { /* Keep the selection for this tab. */ }
    },
    t: (message, values) => translate(locale, message, values),
    formatTime: (timestamp) => formatTime(locale, timestamp)
  }), [locale]);
  return <LanguageContext.Provider value={value}>{children}</LanguageContext.Provider>;
}

export function useI18n() {
  return useContext(LanguageContext);
}

export function LanguageSelect() {
  const { locale, setLocale, t } = useI18n();
  return <label className="language-select"><span>{t("语言")}</span>
    <select value={locale} onChange={(event) => setLocale(event.target.value)}>
      <option value="zh-CN">简体中文</option><option value="en">English</option>
    </select>
  </label>;
}

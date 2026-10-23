import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from './locales/en.json'
import zhCN from './locales/zh-CN.json'

export type AppLanguage = 'en' | 'zh-CN'
export const LANGUAGE_STORAGE_KEY = 'octos.language'

function detectLanguage(): AppLanguage {
  try {
    const stored = localStorage.getItem(LANGUAGE_STORAGE_KEY)
    if (stored === 'en' || stored === 'zh-CN') {
      return stored
    }
  } catch {
    // localStorage can be unavailable in private/embedded contexts.
  }
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en'
}

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    'zh-CN': { translation: zhCN },
  },
  lng: detectLanguage(),
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
  returnNull: false,
})

export function setLanguage(language: AppLanguage) {
  try {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, language)
  } catch {
    // Ignore persistence failures; the in-memory language still changes.
  }
  void i18n.changeLanguage(language)
}

export default i18n

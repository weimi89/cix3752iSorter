import { isEmpty, isEmptyArray, isNullOrUndefined } from './helpers'
import { isNationalIdentificationNumberValid } from 'taiwan-id-validator'

// 👉 必填驗證器
export const requiredValidator = value => {
  if (isNullOrUndefined(value) || isEmptyArray(value) || value === false)
  return '此欄位為必填項目'
  
  return !!String(value).trim().length || '此欄位為必填項目'
}

// 👉 電子郵件驗證器
export const emailValidator = value => {
  if (isEmpty(value))
    return true
  const re = /^(?:[^<>()[\]\\.,;:\s@"]+(?:\.[^<>()[\]\\.,;:\s@"]+)*|".+")@(?:\[\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\]|(?:[a-z\-\d]+\.)+[a-z]{2,})$/i
  if (Array.isArray(value))
    return value.every(val => re.test(String(val))) || '電子郵件欄位必須是有效的電子郵件地址'
  
  return re.test(String(value)) || '電子郵件欄位必須是有效的電子郵件地址'
}

// 👉 密碼驗證器
export const passwordValidator = password => {
  const regExp = /(?=.*\d)(?=.*[a-z])(?=.*[A-Z])(?=.*[!@#$%&*()]).{8,}/
  const validPassword = regExp.test(password)
  
  return validPassword || '此欄位必須包含至少一個大寫字母、小寫字母、特殊字符和數字，並且至少8個字符'
}

// 👉 確認密碼驗證器
export const confirmedValidator = (value, target) => value === target || '確認密碼欄位與密碼不一致'

// 👉 區間驗證器
export const betweenValidator = (value, min, max) => {
  const valueAsNumber = Number(value)
  
  return (Number(min) <= valueAsNumber && Number(max) >= valueAsNumber) || `輸入數字應介於 ${min} 和 ${max} 之間`
}

// 👉 整數驗證器
export const integerValidator = value => {
  if (isEmpty(value))
    return true
  if (Array.isArray(value))
    return value.every(val => /^-?\d+$/.test(String(val))) || '此欄位必須是整數'
  
  return /^-?\d+$/.test(String(value)) || '此欄位必須是整數'
}

// 👉 數字驗證器
export const numericValidator = value => {
  if (isEmpty(value))
    return true
  if (Array.isArray(value))
    return value.every(val => /^[0-9]+(,[0-9]{3})*(\.[0-9]+)?$/.test(String(val))) || '此欄位必須是數字'
  
  return /^[0-9]+(,[0-9]{3})*(\.[0-9]+)?$/.test(String(value)) || '此欄位必須是數字'
}

// 👉 正則表達式驗證器
export const regexValidator = (value, regex) => {
  if (isEmpty(value))
    return true
  let regeX = regex
  if (typeof regeX === 'string')
    regeX = new RegExp(regeX)
  if (Array.isArray(value))
    return value.every(val => regexValidator(val, regeX))
  
  return regeX.test(String(value)) || '正則表達式格式無效'
}

// 👉 字母驗證器
export const alphaValidator = value => {
  if (isEmpty(value))
    return true
  
  return /^[A-Z]*$/i.test(String(value)) || '字母欄位只能包含字母字符'
}

// 👉 URL 驗證器
export const urlValidator = value => {
  if (isEmpty(value))
    return true
  const re = /^https?:\/\/[^\s$.?#].\S*$/
  
  return re.test(String(value)) || 'URL 無效'
}

// 👉 長度驗證器
export const lengthValidator = (value, length) => {
  if (isEmpty(value))
    return true
  
  return String(value).length === length || `最小字符欄位必須至少為 ${length} 個字符`
}

// 👉 字母破折號驗證器
export const alphaDashValidator = value => {
  if (isEmpty(value))
    return true
  const valueAsString = String(value)
  
  return /^[\w-]*$/.test(valueAsString) || '所有字符均無效'
}

// 👉 中文姓名驗證器
export const fullNameValidator = value => {
  if (isEmpty(value))
  return true

return /^[\u4e00-\u9fa5]+$/.test(value) || '請輸入有效的中文姓名'
}

// 👉 手機號碼驗證器
export const phoneNumberValidator = value => {
  if (isEmpty(value))
  return true
const re = /^09\d{8}$/

return re.test(String(value)) || '請輸入有效的手機號碼'
}

// 👉 身分證驗證器
export const idNoValidator = value => {
  if (isEmpty(value))
    return true
  
  return isNationalIdentificationNumberValid(value) || '請輸入有效的身分證字號'
}

// 👉 車牌號碼驗證器
export const carNoValidator = value => {
  if (isEmpty(value))
    return true
  const re = /^([A-Z]{2,4}|\d{2,4})-([A-Z]{2,4}|\d{2,4})$/
  
  return re.test(String(value)) || '請輸入有效的車牌號碼'
}
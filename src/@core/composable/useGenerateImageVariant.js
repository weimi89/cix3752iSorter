import { useTheme } from 'vuetify'
import { useConfigStore } from '@core/stores/config'

// composable function to return the image variant as per the current theme and skin
export const useGenerateImageVariant = (imgLight, imgDark, imgLightBordered, imgDarkBordered, bordered = false) => {
  const configStore = useConfigStore()
  const vuetifyTheme = useTheme()

  return computed(() => {
    if (vuetifyTheme.name.value === 'light') {
      if (configStore.skin === 'bordered' && bordered)
        return imgLightBordered
      else
        return imgLight
    }
    if (vuetifyTheme.name.value === 'dark') {
      if (configStore.skin === 'bordered' && bordered)
        return imgDarkBordered
      else
        return imgDark
    }

    // Add a default return statement
    return imgLight
  })
}

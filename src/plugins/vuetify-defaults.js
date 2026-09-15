// 對齊 Materio/resources/js/plugins/vuetify/defaults.js（Materio 風）
// 維持與既有後台元件大小、顏色、密度一致

export default {
  IconBtn: {
    icon: true,
    color: 'default',
    variant: 'text',
  },
  VAlert: {
    density: 'comfortable',
    VBtn: {
      // v4 合併 defaults 會略過 undefined;要「清掉全域 VBtn 顏色」得用 null
      color: null,
    },
  },
  VAvatar: {
    variant: 'flat',
  },
  VBadge: {
    color: 'primary',
  },
  VBtn: {
    color: 'primary',
  },
  VCard: {
    variant: 'outlined',
  },
  VChip: {
    // 維持 chip 預設
  },
  VDataTable: {
    VPagination: {
      showFirstLastPage: true,
      firstIcon: 'tabler-chevrons-left',
      lastIcon: 'tabler-chevrons-right',
    },
  },
  VDataTableServer: {
    VPagination: {
      showFirstLastPage: true,
      firstIcon: 'tabler-chevrons-left',
      lastIcon: 'tabler-chevrons-right',
    },
  },
  VExpansionPanel: {
    expandIcon: 'tabler-chevron-right',
    collapseIcon: 'tabler-chevron-right',
  },
  VExpansionPanelTitle: {
    expandIcon: 'tabler-chevron-right',
    collapseIcon: 'tabler-chevron-right',
  },
  VList: {
    color: 'primary',
    density: 'compact',
    VCheckboxBtn: { density: 'compact' },
    VListItem: {
      ripple: false,
      VAvatar: { size: 40 },
    },
  },
  VMenu: { offset: '2px' },
  VPagination: { density: 'comfortable', variant: 'tonal' },
  VTabs: {
    color: 'primary',
    density: 'comfortable',
    VSlideGroup: { showArrows: true },
  },
  VTooltip: { location: 'top' },
  VCheckboxBtn: { color: 'primary' },
  VCheckbox: {
    color: 'primary',
    density: 'comfortable',
    hideDetails: 'auto',
  },
  VRadioGroup: {
    color: 'primary',
    density: 'comfortable',
    hideDetails: 'auto',
  },
  VRadio: {
    density: 'comfortable',
    hideDetails: 'auto',
  },
  VSelect: {
    variant: 'outlined',
    color: 'primary',
    density: 'comfortable',
    hideDetails: 'auto',
    VChip: { label: true },
  },
  VRangeSlider: {
    color: 'primary',
    trackSize: 6,
    thumbSize: 22,
    density: 'comfortable',
    thumbLabel: true,
    hideDetails: 'auto',
  },
  VRating: { color: 'warning' },
  VProgressLinear: {
    height: 6,
    roundedBar: true,
    rounded: true,
    bgColor: 'rgba(var(--v-track-bg))',
  },
  VSlider: {
    color: 'primary',
    thumbLabel: true,
    hideDetails: 'auto',
    thumbSize: 22,
    trackSize: 6,
    elevation: 4,
  },
  VTextField: {
    variant: 'outlined',
    density: 'comfortable',
    color: 'primary',
    hideDetails: 'auto',
  },
  VNumberInput: {
    variant: 'outlined',
    density: 'comfortable',
    color: 'primary',
    hideDetails: 'auto',
    controlVariant: 'stacked',
  },
  VAutocomplete: {
    variant: 'outlined',
    color: 'primary',
    density: 'comfortable',
    hideDetails: 'auto',
    VChip: { label: true },
  },
  VCombobox: {
    variant: 'outlined',
    density: 'comfortable',
    color: 'primary',
    hideDetails: 'auto',
    VChip: { label: true },
  },
  VFileInput: {
    variant: 'outlined',
    density: 'comfortable',
    color: 'primary',
    hideDetails: 'auto',
  },
  VTextarea: {
    variant: 'outlined',
    density: 'comfortable',
    color: 'primary',
    hideDetails: 'auto',
  },
  VSnackbar: {
    VBtn: { density: 'comfortable' },
  },
  VSwitch: {
    // 用 Vuetify 原生的開關（非 inset），Materio 的 inset 版在卡片與表單裡都偏大
    inset: false,
    color: 'primary',
    hideDetails: 'auto',
    ripple: false,
  },
  VNavigationDrawer: { touchless: true },
}

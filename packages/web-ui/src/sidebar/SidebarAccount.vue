<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue';
import { accountDisplayName } from '../auth/account-display';
import Avatar from '@demicodes/web-ui/ui/Avatar.vue';
import { CircleUser, LogOut, Settings } from '@lucide/vue';
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay';
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue';
import Menu from '@demicodes/web-ui/ui/Menu.vue';
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue';
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue';
import { pressOnKey } from '@demicodes/web-ui/ui/button-events';
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics';
import { provideRowTarget, type Action } from '@demicodes/web-ui/ui/operations';
import RowSpinner from '@demicodes/web-ui/ui/RowSpinner.vue';
import type { SidebarAccount } from './types';
import { THIS_SESSION } from './session-target';

/**
 * The signed-in user at the foot of the sidebar; the menu behind it holds
 * settings and sign-out. The row takes what its container leaves it, and a
 * name that does not fit beside the avatar ends in an ellipsis, the menu
 * still showing it.
 */
const props = defineProps<{
  account: SidebarAccount;
  /** Signs this browser out; the row works until the page goes to the sign-in page. */
  onSignOut?: Action;
}>();

const emit = defineEmits<{
  openSettings: [];
}>();

// Signing out, from this menu or from Settings, shows on the account's row.
const signingOut = provideRowTarget(() => THIS_SESSION.target);

const displayName = computed(() =>
  accountDisplayName(props.account.name, props.account.email)
);
</script>

<template>
  <!-- The account sits at the sidebar's foot, so its menu opens above it. -->
  <Dropdown
    :overlay-store="appOverlayStore"
    side="top"
    :offset="8"
    width="fill"
  >
    <template #trigger="{ isOpen }">
      <!-- A name too long for the row stays on its one line beside the avatar and
           ends in an ellipsis; the avatar keeps its size. -->
      <div
        role="button"
        :aria-label="t('account.label')"
        tabindex="0"
        class="flex h-9 w-full cursor-default select-none items-center gap-2 rounded-md pl-2 pr-1.5 transition-colors duration-200 ease-out phone:h-11 phone:gap-3"
        :class="isOpen ? 'bg-hover' : 'hover:bg-hover'"
        @keydown="pressOnKey"
        @keyup="pressOnKey"
      >
        <!-- 24 px, and 32 px on a phone, as the row grows to a finger's height. -->
        <Avatar
          class="phone:size-8 phone:text-[13px]"
          :account-id="account.id"
          :name="account.name"
          :email="account.email"
          :size="24"
        />
        <span class="min-w-0 flex-1 truncate text-chrome text-fg phone:text-[16px]">{{ displayName }}</span>
        <RowSpinner v-if="signingOut" />
      </div>
    </template>
    <template #content="{ close }">
      <Menu>
        <div class="flex items-center gap-2 px-2 py-1.5">
          <CircleUser :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
          <span class="flex min-w-0 flex-col leading-4">
            <span class="truncate text-chrome text-fg">{{ displayName }}</span>
            <!-- Without a name the email is what shows above; it is not repeated. -->
            <span
              v-if="account.name.trim() && account.email"
              class="truncate text-[11px] text-fg-subtle"
            >{{ account.email }}</span>
          </span>
        </div>
        <MenuDivider />
        <MenuItem
          :icon="Settings"
          :label="t('account.settings')"
          shortcut="⌘,"
          @select="
            close();
            emit('openSettings');
          "
        />
        <MenuDivider />
        <MenuItem
          :icon="LogOut"
          :label="t('account.signOut')"
          :operation="THIS_SESSION"
          @select="onSignOut?.()"
        />
      </Menu>
    </template>
  </Dropdown>
</template>

<script setup lang="ts">
// The app the phone scene's simulator shows: Harbor, an invented app whose
// sign-up the agent tests, drawn as a page is, in the simulator's 402 × 874
// points, so it stays sharp however close the camera comes.
defineProps<{
  welcome: boolean
  email: string
  /** How many of the password's characters are typed. */
  dots: number
  /** The field the agent types into, if any. */
  typing: 'email' | 'password' | null
  /** Whether Sign Up is pressed. */
  pressed: boolean
  /** Whether the tap's ring shows, as the simulator's touch indicator draws it. */
  ring: boolean
}>()
</script>

<template>
  <div class="screen">
    <b class="clock">9:41</b>
    <div v-if="welcome" class="welcome">
      <svg class="logo" viewBox="0 0 100 100" width="88" height="88" aria-hidden="true">
        <rect width="100" height="100" rx="26" fill="#1f7a74" />
        <path d="M22 42 C38 32 62 52 78 42 M22 62 C38 52 62 72 78 62" fill="none" stroke="#fff" stroke-width="7" stroke-linecap="round" />
      </svg>
      <h1>Welcome to Harbor</h1>
      <p>Your account is ready.</p>
    </div>
    <div v-else class="form">
      <svg class="logo" viewBox="0 0 100 100" width="56" height="56" aria-hidden="true">
        <rect width="100" height="100" rx="26" fill="#1f7a74" />
        <path d="M22 42 C38 32 62 52 78 42 M22 62 C38 52 62 72 78 62" fill="none" stroke="#fff" stroke-width="7" stroke-linecap="round" />
      </svg>
      <h1>Create your account</h1>
      <p>It takes less than a minute.</p>
      <label>Email</label>
      <span class="field" :class="{ focused: typing === 'email' }">{{ email }}<i v-if="typing === 'email'" class="caret" /></span>
      <label>Password</label>
      <span class="field" :class="{ focused: typing === 'password' }">{{ '•'.repeat(dots) }}<i v-if="typing === 'password'" class="caret" /></span>
      <span class="sign-up" :class="{ pressed }">Sign Up<i v-if="ring" class="ring" /></span>
    </div>
  </div>
</template>

<style scoped>
.screen {
  position: relative;
  width: 402px;
  height: 874px;
  background: #f6f8f8;
  color: #1d2427;
  font: 400 17px/1.2 -apple-system, system-ui, sans-serif;
  -webkit-font-smoothing: antialiased;
}
.clock {
  position: absolute;
  left: 40px;
  top: 24px;
  font-size: 16px;
  font-weight: 600;
}
.form {
  position: absolute;
  inset: 110px 24px 0;
  display: flex;
  flex-direction: column;
}
h1 {
  margin: 0;
  font-size: 30px;
  font-weight: 700;
  letter-spacing: -0.01em;
}
.form h1 {
  margin-top: 28px;
}
p {
  margin: 8px 0 0;
  color: #8a9499;
}
label {
  margin: 44px 0 10px 8px;
  color: #8a9499;
  font-size: 14px;
  font-weight: 500;
}
label + .field + label {
  margin-top: 34px;
}
.field {
  height: 52px;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  padding: 0 16px;
  border: 1px solid #dfe4e6;
  border-radius: 12px;
  background: #fff;
}
.field.focused {
  border: 2px solid #1f7a74;
  padding: 0 15px;
}
.caret {
  width: 2px;
  height: 22px;
  margin-left: 2px;
  background: #1f7a74;
}
.sign-up {
  position: relative;
  margin-top: 42px;
  height: 54px;
  border-radius: 14px;
  background: #1f7a74;
  color: #fff;
  font-size: 18px;
  font-weight: 600;
  line-height: 54px;
  text-align: center;
}
.sign-up.pressed {
  background: #155a55;
}
.ring {
  position: absolute;
  left: calc(50% + 40px - 30px);
  top: 27px;
  width: 60px;
  height: 60px;
  margin-top: -30px;
  box-sizing: border-box;
  border: 3px solid #fff;
  border-radius: 50%;
  animation: ring 0.4s ease-out both;
}
@keyframes ring {
  from {
    transform: scale(0.53);
    opacity: 0.9;
  }
  to {
    transform: scale(1);
    opacity: 0;
  }
}
.welcome {
  position: absolute;
  inset: 260px 0 0;
  display: flex;
  flex-direction: column;
  align-items: center;
}
.welcome h1 {
  margin-top: 40px;
}
</style>

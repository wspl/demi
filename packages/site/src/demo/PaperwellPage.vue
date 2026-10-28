<script setup lang="ts">
// The page the watch scene's browser shows: Paperwell, an invented
// office-supply shop, drawn as a page is, so it stays sharp however close
// the camera comes. Its tab is 400 × 520 CSS pixels.
defineProps<{
  /** How many products are in the cart. */
  count: number
  /** The product whose Add to Cart is pressed, if any. */
  pressed: number | null
  /** Whether the cart shows, in place of the order. */
  cart: boolean
}>()

const PRODUCTS = [
  { name: 'Dotted notebook A5', detail: 'Pack of 5', price: '$18.50', tint: '#f3ece2', quantity: 1, total: '$18.50', icon: 'notebook' },
  { name: 'Gel pens, assorted', detail: 'Pack of 12', price: '$9.80', tint: '#e9eef4', quantity: 1, total: '$9.80', icon: 'pens' },
  { name: 'Sticky notes', detail: '12 pads, 3 × 3 in', price: '$11.20', tint: '#fbf2d8', quantity: 1, total: '$11.20', icon: 'notes', short: '12 pads' },
  { name: 'Printer paper', detail: '5 reams, letter', price: '$32.40', tint: '#eff1ed', quantity: 1, total: '$32.40', icon: 'paper' },
] as const
</script>

<template>
  <div class="page">
    <header>
      <span class="brand"><i class="mark" />Paperwell</span>
      <nav v-if="cart"><b>Cart</b><span>Delivery</span><span>Payment</span></nav>
      <span v-else class="basket">
        <svg viewBox="0 0 24 24" width="21" height="21" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="8" cy="21" r="1" /><circle cx="19" cy="21" r="1" />
          <path d="M2.05 2.05h2l2.66 12.42a2 2 0 0 0 2 1.58h9.78a2 2 0 0 0 1.95-1.57l1.65-7.43H5.12" />
        </svg>
        <b v-if="count > 0" class="count">{{ count }}</b>
      </span>
    </header>
    <main v-if="!cart">
      <p class="crumbs">Order history · Reorder</p>
      <h1>Your usual order</h1>
      <p class="lead">Last ordered May 2 · to 18 Harbor Street</p>
      <ul class="grid">
        <li v-for="(product, index) in PRODUCTS" :key="product.name" class="card">
          <span class="picture" :style="{ background: product.tint }">
            <svg v-if="product.icon === 'notebook'" viewBox="0 0 48 60" width="45" height="59" aria-hidden="true">
              <rect x="0" y="0" width="45" height="59" rx="3" fill="#2f5d78" /><rect x="0" y="0" width="6" height="59" rx="2" fill="#244a61" />
              <rect x="15" y="13" width="23" height="7" rx="2" fill="#f4f1ea" />
            </svg>
            <svg v-else-if="product.icon === 'pens'" viewBox="0 0 60 50" width="56" height="46" aria-hidden="true">
              <g stroke-linecap="round" stroke-width="5.5"><path d="M6 24 L44 6" stroke="#3367b8" /><path d="M12 33 L50 15" stroke="#24272b" /><path d="M18 42 L56 24" stroke="#cf4e3c" /></g>
            </svg>
            <svg v-else-if="product.icon === 'notes'" viewBox="0 0 52 54" width="50" height="52" aria-hidden="true">
              <rect x="12" y="2" width="38" height="38" rx="2" fill="#efc24d" transform="rotate(5 31 21)" />
              <rect x="2" y="12" width="40" height="40" rx="2" fill="#f6d872" /><rect x="8" y="22" width="24" height="3" rx="1.5" fill="#c9a04a" /><rect x="8" y="30" width="18" height="3" rx="1.5" fill="#c9a04a" />
            </svg>
            <svg v-else viewBox="0 0 56 46" width="54" height="45" aria-hidden="true">
              <rect x="6" y="1" width="44" height="18" rx="2" fill="#f7f8f6" stroke="#cdd4cf" />
              <rect x="1" y="14" width="54" height="31" rx="3" fill="#fbfcfb" stroke="#cdd4cf" /><rect x="1" y="14" width="54" height="7" rx="2" fill="#2e7a66" />
              <text x="28" y="37" text-anchor="middle" font-size="8.5" font-weight="600" fill="#2e7a66">500</text>
            </svg>
          </span>
          <b class="name">{{ product.name }}</b>
          <span class="detail">{{ product.detail }}</span>
          <span class="buy">
            <b>{{ product.price }}</b>
            <span class="add" :class="{ pressed: pressed === index }">Add to cart</span>
          </span>
        </li>
      </ul>
    </main>
    <main v-else>
      <h1 class="cart-title">Your cart</h1>
      <ul class="lines">
        <li v-for="product in PRODUCTS" :key="product.name">
          <span class="thumb" :style="{ background: product.tint }" />
          <span class="what"><b>{{ product.name }}</b><span>{{ 'short' in product ? product.short : product.detail }}</span></span>
          <span class="quantity">{{ product.quantity }}</span>
          <b class="total">{{ product.total }}</b>
        </li>
      </ul>
      <section class="summary">
        <p><span>Items (4)</span><span>$71.90</span></p>
        <p><span>Delivery · Thursday</span><span>Free</span></p>
        <p><span>Tax</span><span>$5.75</span></p>
        <p class="sum"><b>Total</b><b>$77.65</b></p>
        <span class="pay">Continue to payment</span>
      </section>
    </main>
  </div>
</template>

<style scoped>
.page {
  width: 400px;
  height: 520px;
  overflow: hidden;
  background: #fff;
  color: #202326;
  font: 400 11.5px/1.3 -apple-system, system-ui, sans-serif;
  -webkit-font-smoothing: antialiased;
}
header {
  height: 47px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  border-bottom: 1px solid #e6e8e7;
}
.brand {
  display: flex;
  align-items: center;
  gap: 7px;
  color: #1f4d40;
  font: 700 17px/1 Georgia, 'Times New Roman', serif;
}
.mark {
  width: 20px;
  height: 20px;
  border-radius: 5px;
  background: #2e7a66;
}
nav {
  display: flex;
  gap: 12px;
  font-size: 12px;
  color: #8b9290;
}
nav b {
  color: #202326;
  font-weight: 600;
}
.basket {
  position: relative;
  display: flex;
  color: #1f4d40;
}
.count {
  position: absolute;
  right: -7px;
  top: -6px;
  min-width: 15px;
  height: 15px;
  border-radius: 8px;
  background: #c94f3d;
  color: #fff;
  font-size: 10px;
  line-height: 15px;
  text-align: center;
}
main {
  padding: 0 16px;
}
.crumbs,
.lead {
  margin: 0;
  color: #7c8481;
}
.crumbs {
  margin-top: 16px;
}
h1 {
  margin: 4px 0 3px;
  font: 700 22px/1.1 Georgia, 'Times New Roman', serif;
  color: #1d2422;
}
.cart-title {
  margin: 17px 0 10px;
}
.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  margin: 13px 0 0;
  padding: 0;
  list-style: none;
}
.card {
  display: flex;
  flex-direction: column;
  height: 184px;
  padding: 8px;
  box-sizing: border-box;
  border: 1px solid #e3e6e4;
  border-radius: 9px;
}
.picture {
  height: 92px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 9px;
}
.name {
  font-size: 12.5px;
  font-weight: 600;
}
.detail {
  color: #7c8481;
  margin-top: 2px;
}
.buy {
  margin-top: auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.buy > b {
  font-size: 13px;
  font-weight: 600;
}
.add {
  width: 81px;
  height: 26px;
  border-radius: 5.5px;
  background: #2e7a66;
  color: #fff;
  font-size: 11.5px;
  font-weight: 600;
  line-height: 26px;
  text-align: center;
  transition: background-color 60ms;
}
.add.pressed {
  background: #225b4c;
}
.lines {
  margin: 0;
  padding: 0;
  list-style: none;
}
.lines li {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 55px;
  border-bottom: 1px solid #e8ebe9;
}
.thumb {
  width: 38px;
  height: 38px;
  border-radius: 6px;
}
.what {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.what b {
  font-size: 12.5px;
  font-weight: 600;
}
.what span {
  color: #7c8481;
}
.quantity {
  width: 33px;
  height: 23px;
  box-sizing: border-box;
  border: 1px solid #d9dedb;
  border-radius: 5px;
  line-height: 21px;
  text-align: center;
}
.total {
  width: 58px;
  text-align: right;
  font-size: 12.5px;
  font-weight: 600;
}
.summary {
  margin-top: 13px;
  padding: 12px 13px 10px;
  border: 1px solid #e3e6e4;
  border-radius: 9px;
}
.summary p {
  display: flex;
  justify-content: space-between;
  margin: 0 0 6px;
  color: #5c6461;
}
.summary .sum {
  margin-top: 9px;
  padding-top: 10px;
  border-top: 1px solid #e3e6e4;
  color: #1d2422;
  font-size: 14px;
}
.pay {
  display: block;
  margin-top: 8px;
  height: 34px;
  border-radius: 7px;
  background: #2e7a66;
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
  line-height: 34px;
  text-align: center;
}
</style>

# Release notes

## 2.4.0

The checkout keeps the cart when a coupon empties it, and the sign-in page
waits for the session before it moves on.

- **Cart total** stays right when a coupon covers the whole order.
- **Sign-in** no longer flashes the dashboard before the session is ready.
- **Receipts** show the coupon's code beside its discount.

| Area | Change |
| --- | --- |
| Checkout | Coupons that empty the cart |
| Sign-in | Waits for the session |

```sh
bun run deploy --tag 2.4.0
```

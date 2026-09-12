import { emailSchema } from '@demicodes/product-contracts'

/** Uses the product's normalized email constraint for account fields. */
export function isEmail(value: string): boolean {
  return emailSchema.safeParse(value).success
}

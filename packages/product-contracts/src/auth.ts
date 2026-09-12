import { z } from 'zod'

export const EMAIL_MAX_LENGTH = 254
export const PASSWORD_MIN_LENGTH = 8
export const PASSWORD_MAX_LENGTH = 1024
export const NICKNAME_MAX_LENGTH = 80
export const emailSchema = z
  .string()
  .trim()
  .toLowerCase()
  .pipe(z.email().max(EMAIL_MAX_LENGTH))
export const passwordSchema = z
  .string()
  .min(PASSWORD_MIN_LENGTH)
  .max(PASSWORD_MAX_LENGTH)
export const nicknameSchema = z.string().trim().min(1).max(NICKNAME_MAX_LENGTH)

export const userSchema = z.object({
  id: z.string().min(1),
  email: z.email().max(EMAIL_MAX_LENGTH),
  nickname: z.string(),
  role: z.enum(['master', 'admin', 'user']),
  createdAt: z.iso.datetime({ offset: true }),
})
export const identitySchema = z.object({ user: userSchema })

export const instanceModeSchema = z.enum(['shared', 'isolated'])
export type User = z.infer<typeof userSchema>
export type Role = User['role']
export type InstanceMode = z.infer<typeof instanceModeSchema>

# ~/.bashrc: read by every bash through ~/.profile (managed-hosts.md § First
# use of the home). Unlike Ubuntu's, this file does not return early for a
# non-interactive shell: the lines installers append below (nvm, Bun, pnpm,
# ~/.cargo/env) apply to the commands the runner executes as well as to a
# terminal. Only the interactive niceties are guarded.

case $- in
  *i*)
    HISTCONTROL=ignoreboth
    HISTSIZE=1000
    HISTFILESIZE=2000
    shopt -s histappend
    shopt -s checkwinsize
    PS1='\u@\h:\w\$ '
    if [ -x /usr/bin/dircolors ]; then
      eval "$(dircolors -b)"
      alias ls='ls --color=auto'
      alias grep='grep --color=auto'
    fi
    alias ll='ls -alF'
    alias la='ls -A'
    ;;
esac


zmodload zsh/datetime
_zshist_cmd=""
_zshist_dir=""
_zshist_start=0

_zshist_preexec() {
	_zshist_cmd="$1"
	_zshist_dir="$PWD"
	_zshist_start=$EPOCHREALTIME
}

# Returns $ret so later precmd hooks (e.g. the prompt) still see the real
# exit status.
_zshist_precmd() {
	local ret=$?
	if [[ -n "$_zshist_cmd" ]]; then
		local cmd="$_zshist_cmd" dir="$_zshist_dir" start="$_zshist_start"
		_zshist_cmd=""
		if [[ "$cmd" != \ * ]]; then
			local first="${cmd%%$'\n'*}"
			first="${first%% *}"
			if (( ! ${+HIST_EXCLUDE} )) || [[ ${HIST_EXCLUDE[(ie)$first]} -gt ${#HIST_EXCLUDE} ]]; then
				# Integer assignment truncates the float result.
				local elapsed=0
				local -i ms=0
				(( start > 0 )) && elapsed=$(( EPOCHREALTIME - start ))
				(( ms = elapsed * 1000 ))
				(( start > 0 && elapsed >= 0 && ms == 0 )) && ms=1
				(( ms < 0 )) && ms=0
				print -r -- "$cmd" | zshist add --dir "$dir" --exit $ret --ms $ms
			fi
		fi
	fi
	return $ret
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec _zshist_preexec
# Prepend so we read $? before other precmd hooks (prompt, atuin) clobber it.
precmd_functions=(_zshist_precmd $precmd_functions)

_fhistory_select() {
	local qpwd=${(q)PWD}
	# Branches on $FZF_PROMPT so reloads keep whatever mode ctrl-g selected.
	local reload="if [ \"\$FZF_PROMPT\" = \"Dir> \" ]; then zshist list --dir $qpwd; else zshist list; fi"
	local toggle="if [ \"\$FZF_PROMPT\" = \"Dir> \" ]; then echo \"change-prompt(Global> )+reload(zshist list)\"; else echo \"change-prompt(Dir> )+reload(zshist list --dir $qpwd)\"; fi"
	# Preview visibility persists across sessions via a flag file.
	local pstate="${XDG_STATE_HOME:-$HOME/.local/state}/zshist/preview-hidden"
	mkdir -p "${pstate:h}"
	local qstate=${(q)pstate}
	local pwin="down,6,wrap"
	[[ -f "$pstate" ]] && pwin="down,6,wrap,hidden"
	local id
	# Clear the user's fzf defaults so zshist renders the same on every machine.
	id=$(zshist list |
		FZF_DEFAULT_OPTS= FZF_DEFAULT_OPTS_FILE= \
		fzf --ansi --reverse --prompt="Global> " --tiebreak=index \
			--tabstop=1 --delimiter='\t' --with-nth=2.. \
			--preview="zshist get --id {1}" --preview-window=$pwin \
			--header="ctrl-g: dir/global · ctrl-/: preview" \
			--bind "tab:accept" \
			--bind "ctrl-/:toggle-preview+execute-silent(if [ -f $qstate ]; then rm -f $qstate; else touch $qstate; fi)" \
			--bind "ctrl-g:transform:$toggle" |
		cut -f1)
	[[ -n "$id" ]] && zshist get --id "$id"
}

_fhistory_widget() {
	if [[ -n "$BUFFER" && ("$KEYS" == $'\e[A' || "$KEYS" == $'\eOA') ]]; then
		zle up-line-or-history
	elif [[ -n "$BUFFER" && ("$KEYS" == $'\e[B' || "$KEYS" == $'\eOB') ]]; then
		zle down-line-or-history
	else
		local selected
		selected=$(_fhistory_select)
		if [[ -n "$selected" ]]; then
			BUFFER="$selected"
			CURSOR=${#BUFFER}
		fi
		zle reset-prompt
	fi
}
zle -N _fhistory_widget
bindkey '^R' _fhistory_widget

_zshist_search_step() {
	local dir=$1
	local prefix="$LBUFFER"
	if [[ "$LASTWIDGET" != _zshist_search_backward && "$LASTWIDGET" != _zshist_search_forward ]] ||
	   [[ "$prefix" != "$_zshist_search_prefix" ]]; then
		_zshist_search_prefix="$prefix"
		_zshist_search_matches=("${(@f)$(zshist search -- "$prefix")}")
		_zshist_search_index=0
	fi
	if [[ "$dir" == backward ]]; then
		(( _zshist_search_index < ${#_zshist_search_matches} )) && (( _zshist_search_index++ ))
	else
		(( _zshist_search_index > 1 )) && (( _zshist_search_index-- ))
	fi
	if (( _zshist_search_index >= 1 && _zshist_search_index <= ${#_zshist_search_matches} )); then
		BUFFER="${_zshist_search_matches[_zshist_search_index]}"
		CURSOR=${#prefix}
		# zsh-autosuggestions only clears its ghost suggestion on widgets it wrapped at
		# load time; these widgets are defined later by "zshist init" and go unwrapped,
		# so the stale suggestion would otherwise render concatenated onto the new buffer.
		(( $+widgets[autosuggest-clear] )) && zle autosuggest-clear
	fi
}
_zshist_search_backward() { _zshist_search_step backward }
_zshist_search_forward() { _zshist_search_step forward }
zle -N _zshist_search_backward
zle -N _zshist_search_forward
bindkey '^P' _zshist_search_forward
bindkey '^N' _zshist_search_backward
